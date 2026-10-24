# OMP-style prompt composer upgrade: typing system, @-mention file discovery, and n-gram TAB word completion

Dated-frozen: published `2026-10-24`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

## Mechanism

Three operator asks, all modeled on oh-my-pi (OMP, at
`../external-repos/oh-my-pi` — reference, not a dependency):

1. **Typing system** — CTRL+LEFT/RIGHT word-wise shortcuts and OMP's
   typing-related editing affordances in operant's composer.
2. **`@` mention popup** — `@` opens a slash-command-styled popup offering
   files/folders; the chosen reference is injected into the LLM context.
   Indexing uses the OMP mechanism (cached, gitignore-aware recursive walk +
   fuzzy scoring), not a persistent index database.
3. **TAB word completion** — prediction over the corpus of every previously
   submitted prompt (OMP's statistical n-gram engine architecture — the
   "ML/BM25-like" ranker OMP actually ships), surfaced as ghost text that TAB
   accepts.

### Terrain — oh-my-pi (the reference)

**Typing system.** Canonical keybinding registry
(`packages/tui/src/keybindings.ts`): IDs like `tui.editor.cursorWordLeft`,
defaults `ctrl+left / alt+left / alt+b` (word left), `ctrl+right / alt+right /
alt+f` (word right), `ctrl+w / ctrl+backspace / alt+backspace` (delete word
back), `alt+d / ctrl+delete / alt+delete` (delete word fwd), `ctrl+u`/`ctrl+k`
(delete to line start/end), `ctrl+y`/`alt+y` (yank / yank-pop), `ctrl+-`
(undo), `ctrl+]` (jump to char), user-overridable with conflict detection.
Word movement (`moveWordLeft`/`moveWordRight`, `packages/tui/src/utils.ts`)
is grapheme-cluster based with four classes — whitespace, word
(letters/digits/`_`), delimiter (punct/symbol runs move as one stop), CJK
(each segment its own stop) — plus **joiners** (`'` `’` `-` `‐` `‑` stay
inside words: `don't`, `well-known` are one stop). Editor is multi-line
(lines array + cursorLine/cursorCol), `packages/tui/src/components/editor.ts`
(5,019 lines) is the buffer + dispatch; `kill-ring.ts`, `vim.ts`,
`bracketed-paste.ts`, `latex-to-unicode.ts` are the typing accessibility
affordances.

**@-mention + indexing.** `packages/tui/src/autocomplete.ts`: the `@` token
(`@`, `@"quoted path"`, `@dir/` scoping) feeds the SAME `AutocompleteItem`
pipeline as slash commands — one popup component, one keyboard model
(up/down/enter/tab/esc). Two-stage results: (a) immediate directory listing
(`readdir`, dirs-first, `.git` skipped, symlinks stat'd); (b) scoped recursive
fuzzy walk via the `fuzzyFind` native (`crates/pi-natives/src/fd.rs`) with
`maxResults: 100`, `timeoutMs: 1500`, `gitignore: true`, `hidden: true`,
`cache: true`, an interim listing reported after 150 ms (`onPartial`), and
`AbortSignal` cancellation; a stale list is narrowed synchronously with a
subsequence filter while the fresh walk runs. Scoring
(`score_fuzzy_path`): exact filename 100 / prefix / includes / filename
subsequence, falling back to full-path subsequence; **+10 directory bonus**;
ties prefer shallower paths; top-K kept in a worst-first heap DURING the walk
(`TopMatches`), so traversal can early-exit. Quoting: paths with spaces
insert as `@"a b"`; directory values end `/` and chain (no trailing space —
popup stays open to drill down); file values append a space.

**The index is NOT a database** — it is `crates/pi-walker`
(`lib.rs` 6,138 + `cache.rs` 1,115): a parallel directory walker (4 workers,
bulk `getdents64` reads on Linux, gitignore via globset, `.git` skipped,
symlink policies, caller heartbeat) fronted by a shared scan cache keyed
`(root, walk options)` → `Arc<Vec<entry>>`, TTL 1 s
(`FS_SCAN_CACHE_TTL_MS`), empty-result recheck 200 ms, entry/byte caps,
`invalidate_path` on writes. Fuzzy scoring runs over the cached scan, never
over a stored index.

**History + prediction.** Prompt history lives in SQLite `history.db`
(`packages/coding-agent/src/session/history-storage.ts`): unique *normalized*
prompts (`id, prompt, created_at, cwd, session_id, use_count`) with a
full-text index; deduped on resubmission. Prediction is `crates/pi-predict`:
a `Predictor` trait — `complete(before, prefix) → {suffix, confidence}`,
`observe(prompt)` (learn from every submitted prompt), `feedback(query,
suggestion, accepted)`, `persist()` — with three engines (n-gram default
"auto", SmolLM2-135M GGUF opt-in, macOS NSSpellChecker). The n-gram engine
(`pi-predict/src/ngram/`) is an **interpolated absolute-discounting word
trigram model over the user's prose history**: prose-only vocabulary
(editor-side gates exclude code/paths/commands), lexicographic word ids so a
typed prefix is one id range, Fenwick tree for unigram mass + max segment
tree for top-k, id-sorted follower (bigram) lists, open-addressing trigram
table, a prompt-local cache (`exp(−distance/D)`, boosted after the same
previous word), a session cache, a case channel, hygiene (rare
out-of-dictionary words need `min_count` typed hits; edit-distance-1
misspelling folding; pastes never make a word suggestible), show threshold
τ = 0.15, posterior formula in `ngram/query.rs` (score ratios over every
word under the prefix INCLUDING the prefix itself, so typing more letters of
a shown ghost never reorders it). Persistence: versioned zstd snapshot,
atomic temp+rename. Measured: `complete` p50 ≈ 4 µs, `observe` ≈ 0.03 ms,
~52 MiB heap at 58 k prompts. TUI side (`packages/tui/src/prompt/word-completion.ts`,
`typing-word.ts`): the query is the prose word ending at the cursor
(`[\p{L}\p{M}']+`, prose-gated so no ghost inside code/paths/commands);
rendering is synchronous against a 256-entry cache; a cache miss schedules a
single-flight request (one active, newest queued) and repaints on arrival;
**projection** keeps a ghost visible while the user types through it; **TAB
accepts with a trailing space, → accepts without, typing past rejects** and
reports feedback. The engine runs on a dedicated thread behind a JSONL/unix-
socket daemon in OMP; operant will run it in-process instead (below).

### Terrain — operant (current state)

- Composer buffer: single `String` + byte cursor (`prompt_input/state.rs`);
  editing ops in `prompt_input/editing.rs`. **CTRL+LEFT/RIGHT word movement
  already exists** (`app/key_handling.rs` ~1583–1610: Ctrl→`move_word_*`,
  Super→buffer extremes, Shift→jcode selection), as do Alt+B/F, Ctrl+F,
  Alt/Ctrl+Backspace, Alt/Ctrl+Delete, Alt+D, Ctrl+W (kill word), Ctrl+U
  (kill to line start), Ctrl+K, Ctrl+E (line end), Ctrl+Y (yank), Ctrl+Z
  (burst undo), Ctrl+R (history overlay), kill ring (`kill_ring.rs`),
  stash, queued recall. Ctrl+A is the model picker (documented, keep).
  Word class is `is_word_char = alphanumeric || '_'`
  (`prompt_input/vim.rs:132`) with a 3-class scan (word / punct-run /
  whitespace) — **no joiners (`don't` splits), no CJK class**. Home/End are
  BUFFER-scoped (cursor=0/len) while OMP is line-scoped; multi-line drafts
  (Shift+Enter) make buffer-scope surprising.
- Suggestions: `compute_typeahead` (`prompt_input/typeahead.rs`) — slash
  commands (usage-ranked via `slash_usage`) and `@` FileRef share ONE
  pipeline and ONE popup (`operant_ui/ui_input.rs::
  draw_command_suggestions_overlay`), refreshed synchronously on every
  composer mutation via `refresh_prompt_input` (`app/prompt.rs:211`), gated
  by open modals. **The @ flow today**: `suggest_files` does a single
  blocking `fs::read_dir` of one directory, case-insensitive prefix match
  only, no recursion, no gitignore, no `.git` filter, no fuzzy, no cache, no
  quoting; the popup style is already the slash style (ask #2's UI is done).
- **Coherence gap (ask #2's actual bug)**: accepted FileRefs insert bare
  `@src/main.rs`, but the agent-side injection grammar
  (`operant-core/src/context_references.rs`, `REFERENCE_PATTERN`) only
  expands `@file:path`, `@folder:dir`, `@git:…`, `@url:…` — ported from
  hermes with a security model (allowed-root containment, secret-file
  fail-closed list, 50%-of-context-window hard budget). Bare `@path` tokens
  are never injected; the model just sees text. The popup must insert the
  grammar the expander reads, or expansion must grow a bare form — this
  plan inserts `@file:`/`@folder:` (decision D2).
- Prompt history: `~/.operant/history.jsonl` (via `operant_home()`),
  `{ts, text}`, 5,000-entry cap, append on submit with head-dedupe
  (`tui/input_history.rs`); in-memory up/down cycling (`prompt_input/
  history.rs`); Ctrl+R overlay with subsequence scoring
  (`overlays/history_search.rs`). Sessions DB already has FTS5
  (`messages_fts`, `operant-core/src/database.rs:450`). **No word
  prediction exists.**
- Keybinding catalogue (not dispatcher): `tui/keybindings/defaults.rs`
  (`BindingContext::Prompt`) — new chords must be catalogued there for
  `/keys` honesty.
- Deps available: `parking_lot`, `rusqlite (bundled)`, `serde_json`.
  No rayon/walkdir/ignore/globset/zstd/bincode in the workspace.

## Design decisions

- **D1 — word classes, not new chords, are the typing upgrade.** The chords
  already exist; what OMP has that operant lacks is the word-boundary
  semantics: joiners kept inside words, CJK per-segment stops, punctuation
  runs as one stop (operant has the punct-run class already). Port the class
  model into a `word_nav` helper (scoped `char`-based; grapheme clusters are
  out of scope v1 — Rust `char` iteration covers the composer's realistic
  content, and the existing ops are char-based). Keep operant's
  move-word-forward stop position (next word start, readline-style); OMP
  stops at word end — not worth churning shipped behavior. Home/End become
  line-scoped (Super+Left/Right already cover the buffer extremes). Alt+Y
  stays copy-mode (operant's binding); yank-pop is not ported (conflict).
- **D2 — the @ popup inserts the injection grammar.** Trigger tokens:
  `@`, `@file:`, `@folder:` (typed progressively; `@f`→ still fuzzy-matches
  paths, kind chosen by the accepted entry). Accepted file → insert
  `@file:<path> ` (trailing space, popup closes); accepted directory →
  insert `@folder:<path>/` and KEEP the popup open drilling into it. Paths
  with spaces insert quoted (`@file:"a b"` — `REFERENCE_PATTERN` already
  parses quoted values). Bare-`@path` support is NOT added to the expander:
  the expander's allowlist grammar and secret-file fail-closed list are
  load-bearing (AGENTS: context_references mirrors hermes R1); widening the
  accepted token surface from the TUI side is the safer direction.
- **D3 — OMP-style indexing = cached walk + fuzzy scoring, no index store.**
  New module `crates/operant-cli/src/tui/file_discovery/` (walker + scan
  cache + scorer). Gitignore/hidden filtering via the `ignore` crate (the
  one new dependency; hand-rolling gitignore semantics is a bug farm, and
  `ignore` is the de-facto standard `ripgrep` walker). Cache: global
  `(root, options) → Arc<Vec<DiscoveredEntry>>`, TTL 1 s, empty-recheck
  200 ms, entry cap, invalidated on accept (paths change). Walk is
  single-threaded v1 with the OMP budget shape — deadline 1.5 s, top-100
  worst-first heap scored DURING traversal (early exit at cap+slack),
  +10 directory bonus, shallower-path tiebreak; parallelism is a measured
  follow-up, not a default (repo doctrine: no perf claim without a number;
  the heap+deadline makes the single thread adequate for repo-scale trees).
- **D4 — prediction = the OMP n-gram architecture, in-process, history-only
  prior.** Port the *architecture* (interpolated absolute-discounting
  trigram, lexicographic-id prefix ranges, follower lists, trigram table,
  prompt-local + session caches, case channel, hygiene, posterior with
  prefix-as-word mass, τ=0.15, snapshot persistence) — implemented natively
  in `crates/operant-cli/src/tui/prompt_input/prediction/` (operant Rust,
  `thiserror`-free simple Result, no `anyhow` per repo law). v1 prior is the
  user's own history (no embedded Norvig web file, no SmolLM, no daemon):
  bootstrap ingests `history.jsonl`, `observe()` on every submit (hooked
  next to `input_history::append`). Web prior and SmolLM are explicit
  non-goals (2.6 MB data / GGUF runtime). Snapshot: `~/.operant/predict/
  ngram.snapshot`, serde_json v1 (zstd only if the snapshot measurably
  matters), atomic temp+rename. Engine runs on one background thread behind
  an mpsc — same single-flight + newest-queued semantics as
  `word-completion.ts` — fed from the TUI event loop; no daemon process.
- **D5 — ghost text obeys the composer's laws.** Ghost renders ONLY in the
  composer input line (dim suffix after cursor at end of the word), never in
  the transcript (one-renderer law); suppressed while the suggestion popup is
  open or the query is non-prose (inside code spans, paths, URLs,
  slash-command or @-ref tokens — port the prose gate minimally: no ghost
  when the char before the word is `/` `\` `` ` `` or the token is mid-@ref).
  TAB: popup accept wins if the popup is open; else accept ghost + trailing
  space. → accepts without space. Typed-past diverge rejects + feedback.
  Projection keeps a still-consistent ghost while typing through it.

## Milestones (each = one iteration: scoped check + tests + clippy gate + commit + push + release deploy)

**M1 — word-class + line-scope typing upgrade** (`fix(iter-N)`)
- `prompt_input/word_nav.rs`: classify(char) → Whitespace|Word|Delimiter|Cjk;
  joiner set `'` `’` `-` `‐` `‑` binds between word chars; rewrite
  move/kill/delete word ops in `editing.rs` on it (keep signatures; the
  `is_word_char` call sites in editing.rs switch to the new scan).
- Home/End → line-scoped in `app/key_handling.rs` (rfind/find '\n').
- Catalogue trueness: `/keys` entries for any chord whose behavior changed.
- Verify: new unit tests in `prompt_input/tests.rs` (joiner words, CJK run,
  punct run, line-scoped Home/End on multi-line drafts); existing suite
  green; `cargo check -p operant-cli --bin operant`; clippy gate.

**M2 — file discovery engine, OMP style** (`feat(iter-N)`)
- `tui/file_discovery/{mod,walker,cache,score}.rs` per D3; `ignore` crate
  added to `operant-cli` deps (workspace + crate manifest).
- `DiscoveredEntry { rel_path, is_dir, depth }`; `discover(root, opts)` →
  cached walk; `fuzzy_top(root, query, limit, deadline)` → ranked entries
  (port OMP scoring: exact/prefix/includes/subsequence on filename, then
  path subsequence, +10 dir, depth tiebreak).
- Immediate-directory listing moves here from `typeahead.rs::suggest_files`,
  gaining `.git` skip + symlink stat + hidden policy.
- Verify: unit tests with a temp-tree fixture (gitignore honored, .git
  skipped, hidden filtered, dir bonus, deadline early-exit on a synthetic
  large tree ≥ 10k entries under 1.5 s); scoped `-p operant-cli` check;
  clippy gate (no `unwrap` in production paths).

**M3 — async suggestion pipeline** (`feat(iter-N)`)
- `refresh_prompt_input` keeps slash computation synchronous (cheap,
  registry-only) and dispatches file discovery to a background worker with a
  generation counter: single-flight, newest-queued; results return via mpsc
  drained on the TUI event tick; stale generation dropped; the previous
  list is narrowed synchronously (port `atCompletionMatches` subsequence
  filter) so every keystroke shows something coherent; interim immediate-
  directory results after the OMP 150 ms shape (not a hard timer — first
  sync listing IS the interim).
- Remove the blocking `fs::read_dir` from the UI path (the old comment in
  `typeahead.rs:213` named this stall).
- Verify: unit tests for generation/staleness/narrowing; a
  `tui debug simulate` scenario driving `@crates/` keystrokes asserting the
  popup appears without frame stall (`--assert-screen`); corpus entry.

**M4 — @file:/@folder: grammar alignment + chaining + quoting** (`fix(iter-N)`)
- Trigger/accept per D2 in `typeahead.rs`/`suggestions.rs`: parse `@`,
  `@file:`, `@folder:` tokens up to cursor; accept inserts grammar tokens;
  directory accept chains (popup stays open on the new prefix); file accept
  appends space (Tab arm gains what `accept_suggestion_for_submit`'s
  ExtendInput already does); quoted values for spaces.
- `has_active_file_ref` recognizes all three trigger forms at word boundary.
- Verify: unit tests (accept file/dir/quoted; drill-down; text after cursor
  preserved); an end-to-end test asserting the inserted token round-trips
  through `context_references::preprocess_context_references` and injects
  content (this is the ask-#2 acceptance test); corpus scenario.

**M5 — n-gram prediction engine** (`feat(iter-N)`)
- `prompt_input/prediction/{mod,model,query,snapshot}.rs` per D4:
  `Predictor` trait, trigram model with the OMP structures (sorted-id vocab,
  prefix ranges, followers, trigram hash table, caches, case channel,
  hygiene `min_count`, τ gate), `observe`/`persist`, bootstrap from
  `history.jsonl`, engine thread + mpsc service.
- Config: `[tui] word_completion = "off" | "ngram"` (default `off` for the
  landing iteration, flipped to `"ngram"` after M6 live verification) +
  `word_completion_threshold` override; `operant.example.toml` updated in
  the same change (AGENTS rule).
- Verify: port a compact version of OMP's replay methodology on the local
  `history.jsonl` (train on all but last 200, replay: KSR % and wrong-ghost
  rate recorded in the plan's run log — numbers or it didn't happen);
  property tests (typing more letters never reorders a shown ghost — the
  posterior's ratio property); snapshot round-trip test; corrupt-snapshot
  wipe-and-reingest test.

**M6 — ghost text in the composer** (`feat(iter-N)`)
- Prose gate + query extraction (`word_completion_query` port, char-based);
  ghost state on `PromptInputState` (suffix, generation); render: dim ghost
  after cursor in the composer line only (`operant_ui/ui_input.rs`), cursor
  drawn before the ghost; TAB/→/typed-past semantics per D5 with feedback
  into the engine thread; projection while typing through; popup-open
  suppression; `App` wiring (single-flight request on word change).
- Verify: unit tests (gate cases: code span, path, @ref, slash token; accept
  with/without space; diverge-reject); `tui debug simulate` scenarios
  (`--assert-screen 'count:<ghost>==1'`) with a seeded snapshot fixture;
  corpus entry; live tmux verification per AGENTS TUI debug protocol
  (rebuild debug binary first — stale-cache false readings bit twice).
- Flip default to `"ngram"` in the same iteration if live verification
  passes; else leave `off` and record why.

**M7 — history store enrichment (BM25 corpus quality)** (`feat(iter-N)`, optional but recommended)
- `history.jsonl` gains `{ts, text, cwd, use_count}` with OMP-style
  normalization (CRLF fold, line-trail strip, trim) + dedupe-by-content
  bumping `use_count` — the prediction corpus and Ctrl+R overlay both read
  it; back-compat read of the old 2-field lines.
- Verify: migration round-trip test; engine re-bootstrap on upgrade path.

**M8 — docs + ledger closeout** (`docs(iter-N)`)
- AGENTS.md (TUI section: the three surfaces + their laws), README
  composer keymap, `docs/CHANGELOG.md`, BUGS.md rows if any surface issues
  were found, `/keys` catalogue audit (`keybindings/defaults.rs` matches
  shipped dispatch), screenshots if the popup/ghost changed the look.

## Gates / success criteria

(fixed before the run)

1. `@` → popup (slash-styled) → accepted dir chains, accepted file inserts a
   token that `preprocess_context_references` demonstrably injects (M4
   round-trip test is the gate for ask #2).
2. Typing `@<fuzzy>` surfaces files from NESTED directories (recursion gate —
   today's single-dir listing cannot), gitignored paths never appear, and
   the UI never blocks on the walk (async gate, M3 scenario).
3. CTRL+LEFT/RIGHT and friends move by OMP word classes: `don't` and
   `well-known` are single stops, punctuation runs are single stops, CJK
   walks per segment (M1 unit gates).
4. With ≥ 500 prompts of history, TAB completes a word mid-type from the
   personal corpus as ghost text; ghost survives typing-through; wrong-ghost
   rate measured and recorded (M5/M6 gates); engine latency never stalls a
   keystroke (background-thread gate).
5. Every milestone: scoped `check.sh` + crate tests + clippy gate green,
  committed as its own iteration, pushed to origin/main, release binary
  deployed (R&D protocol), corpus `verify` gates updated where scenarios
  were added.

## Cost

- Reference reading is done (this plan); implementation is 8 iterations
  (M1–M8), each independently shippable. Heaviest: M2 (walker+cache, ~600
  lines + tests) and M5 (engine, ~900 lines + replay harness). One new
  dependency (`ignore`). No operant-core changes except none required —
  @file:/@folder: expansion already exists untouched; all work is
  operant-cli + docs.
- Risks: async suggestion plumbing vs the single-flight tick loop (mitigated
  by generation counters + sync narrowing); ghost text vs prompt-cache
  stability (none — composer rendering is post-turn-context, the frozen
  prefix is untouched); prediction cold-start on thin history (τ gate shows
  nothing rather than junk — by design); `ignore` crate pull (small,
  pure-Rust, ripgrep's walker).
