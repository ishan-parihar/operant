# close the key-hint drift iter-2714 introduced and guard it

Dated-frozen: published `2026-11-20`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

## Why this plan exists

iter-2714 (`feat(iter-2714): close the TUI audit round's verified findings`) gave the
vim `j`/`k` pair to five move-only surfaces. It updated the DISPATCH and added a guard
for the dispatch. It did not update the HINTS those surfaces paint, and the guard it
added asserts only the dispatch direction (arrows scroll) — never that the painted
claim matches what the handler now accepts.

So the change left the exact defect class this repo has now filed twice, in the same
area, with the same shape:

- **RT-273** (CLOSED iter-1840): the diff viewer's footer advertised `d toggle scope`
  — a key with NO arm anywhere. The operator pressed it and nothing happened.
- **RT-366** (CLOSED): `BindingContext::Dialog` claimed `Enter` = "Confirm" for every
  dialog, and the memory file selector's Enter arm was byte-for-byte its Esc arm.

iter-2714 is the MIRROR of both: real keys the surface now accepts, that its own
footer does not name. RT-273 was "footer names a key that does nothing"; this is
"key does something the footer does not name". The operator's failure mode is the
same — they read the footer, they press what it says, and they never discover `j`/`k`
exists. That is an AESTHETIC defect (the footer is wrong) and an OPERATIONAL one (a
whole key dialect is undocumented on the surfaces that just gained it).

**This plan does not re-litigate T3's exclusion set.** Five other arrows-only surfaces
were checked and are correctly excluded because each has a live text field where `j`/`k`
must stay literal:

| surface | why `j`/`k` is correctly NOT bound |
|---|---|
| `global_search` | `handle_global_search_key` has `KeyCode::Char(c) => push_char(c)` — it IS a query field (`global_search.rs:17`) |
| `agent_picker` | `KeyCode::Char(c) => filter_push(c)` (`key_handling.rs:650`) |
| `command_palette` | shares `DialogSelectState` + its filter field (`dialog_select.rs`) |
| `import_config_picker` | same shared state, same filter field |
| `ask_user_dialog` | a text answer surface by definition |

The one shared component in that table — `render_dialog_select` — paints
`hint: HINT_ESC` (`"Esc to close"`) and `footer_height: 0`, so it advertises NO
navigation keys at all. That is a GAP (T6 below), not a lie: nothing it says is
false.

## The tranches

### T1 — Repair the three stale hints (the direct residue of iter-2714)

Each is a literal string the surface paints, and each now under-describes the handler.

| file | line | now says | must also say |
|---|---|---|---|
| `tui/memory_file_selector.rs` | 217 | `"  ↑↓ navigate  Esc close"` | `j`/`k` navigate |
| `tui/diff_viewer/render.rs` | 36 | `" tab switch pane  ·  ↑↓ navigate  ·  pgup/pgdn/wheel scroll"` | `j`/`k` navigate |
| `tui/stats_dialog/render.rs` | 55 | `" tab/←/→ switch tabs  ·  r cycle range  ·  ↑↓ scroll"` | `j`/`k` scroll |

Verified against the handlers, not assumed:

- `memory_file_selector` — `KeyCode::Up | KeyCode::Char('k')` / `Down | Char('j')`
- `diff_viewer` — same, both panes (`diff_viewer/render.rs:29` documents it)
- `stats_dialog` — `handle_stats_dialog_key` (`key_handling.rs:2910`) has
  `Up | Char('k')` and `Down | Char('j')` around line 2913

**Two surfaces that need NO change, and why that is worth stating:** `help_overlay`
already paints `" Esc to close · drag to select, release to copy · wheel/j/k scroll "`
(`operant_ui/ui_overlays.rs:84`) and `key_handling.rs:3069/3078` does bind
`Up | Char('k')` / `Down | Char('j')` — honest, because T3 added the arms AND the
string it already advertised was the true one. `subagent_navigator` paints
`"t view · Enter focus · Esc close"` (`subagent_navigator.rs:191`): it names three
keys the operator did NOT already know and omits the navigation pair. Judgement: a
hint naming three specific non-obvious keys is doing its job; adding a fourth clause
for a key pair the operator can guess is noise. **Record the decision rather than
silently changing it** — and say so in the commit body so the next auditor does not
re-file it.

### T2 — Make the hint DRIFT detectable, not just this instance

The guard iter-2714 added (`diff_viewer/tests.rs:320`,
`the_diff_viewers_footer_only_names_keys_it_dispatches`) is a hand-written table of
`&[(&str, &str, &str)]` claims inside the diff_viewer's own test module. It catches
diff_viewer drift and nothing else. RT-273 and RT-366 both got their own bespoke
pins; the class has now recurred three times with three separate tests, which is the
signature of a missing SEAM rather than a missing test.

Add one test that drives the seam every surface shares — the footer strings are
literals and the handler arms are literals, so the honest check is a single table
over the T3 surfaces:

```
(footer_literal, must_contain, handler_file)
("↑↓ navigate  Esc close", "j/k", "memory_file_selector")
(FOOTER_HINT,               "j/k", "diff_viewer")
("↑↓ scroll",               "j/k", "stats_dialog")
```

**Do not over-engineer this into a source parser.** The precedent in this repo is
`include_str!` + a claim table (RT-273's pin, RT-366's `esc_closes_every_dialog`).
Follow it. The test asserts TWO directions so it fails on either drift:
footer names a key the handler lacks (RT-273's shape), and the handler accepts a
key the footer omits (iter-2714's shape).

### T3 — The `/keys` Dialog bucket after iter-2714

`BindingContext::Dialog` carries exactly ONE row — `Esc` = "Close dialog"
(`keybindings/defaults.rs:1096`). RT-366 removed the over-claiming `Tab`/`Enter` rows
deliberately, on the reasoning that "every dialog that DOES confirm advertises it in
its own surface" (`export_dialog.rs:106`, `import_config_dialog.rs:100`,
`agents_view.rs:550` verified). That reasoning still holds and this plan does NOT
re-add a generic `Enter` row.

What iter-2714 changed is that `j`/`k` are now a CROSS-DIALOG dialect on five
surfaces (memory selector, diff viewer, stats, subagent navigator, help). The Dialog
bucket is the one place `/keys` could say so once instead of five times — and a
bucket row is a claim about every dialog, so it must be worded to survive the
counter-examples (`ask_user_dialog`, `command_palette`, `global_search`,
`agent_picker`, `import_config_picker` all keep `j`/`k` literal). Candidate wording:
`j/k` = "navigate (lists only — dialogs with a text field keep j/k literal)".
`esc_closes_every_dialog` (`app/tests.rs`) is the pattern to extend, and it must be
extended rather than replaced: the new row needs its own mutation-proven test that
REMOVING the qualifier fails.

### T4 — `render_dialog_select` advertises no navigation at all

Five surfaces render through it (`command_palette`, `agent_picker`,
`import_config_picker`, `permissions_picker`, and one more via
`paint_agent_picker`/`paint_command_palette`), and every one paints
`hint: HINT_ESC` = `"Esc to close"` with `footer_height: 0`. The filter field IS
drawn (a search row under the title), so the surface invites typing, and the arrows
are invisible.

Judgement call, and it belongs to the operator: adding a hint clause here changes
five surfaces' chrome at once, and `DialogSelectState`'s items are a
filter-and-select list where the arrows are the least surprising thing in the UI.
**Recommendation: no change.** The bar this plan holds itself to is "no claim on
screen is false", and `HINT_ESC` is true. File the observation, do not patch it.

### T5 — Route this plan's own findings to the ledger

The three hint rows close as one new RT row (they are one defect: one change, three
sites), with the T2 seam and the T3 bucket row named in the closure. Do not create
three rows for one cause — that is the fragmentation this audit is meant to be
reducing.

## Status: executed iter-2716 (T1, T2, T5 done; T3, T4 declined with reasons in the ledger row RT-617)

## Gates / success criteria

(fixed before the run)

1. **No painted hint is false.** Grep-derived table over every surface iter-2714
   touched; each hint names a key its handler dispatches. Machine-checkable via the
   T2 test.
2. **The guard fails RED without the fix.** Mutation-prove: revert
   `memory_file_selector.rs:217` to the arrows-only string, the T2 test must fail by
   name. This is the repo's rule for every fix, and it is the one iter-2714 skipped
   for its own residue.
3. **`cargo fmt --check` clean on the touched files only** (rule 3 — do not
   `cargo fmt --all` and sweep a peer's unformatted file into my commit; iter-2714
   had to revert exactly that).
4. **`cargo check -p operant-cli --bin operant --tests`** (the AGENTS §Compile
   Strategy note: `--bin` alone does not type-check `#[cfg(test)]`).
5. **`cargo test -p operant-cli`** green, with any pre-existing failure PROVEN
   pre-existing by running the same filter on the untouched tip (iter-2714's
   method — 6 TUI + 1 config failure were pre-existing and were not mine).
6. **`./scripts/check.sh check -p operant-core --lib`** if any operant-core file is
   touched (none planned).
7. **Iteration number read from `origin/main`** after a fetch, never from a header or
   local history (the claim rule; iter-2714 landed as `iter-2714` after two rebases).

## Out of scope (owned elsewhere — do not duplicate)

- **`2026-11-20-close-the-consolidation-gaps-*`** — clippy gate, genome findings,
  ledger index.
- **`2026-11-20-close-the-remaining-consolidation-gaps-*`** — config-key ratchet,
  supply-chain advisories, pub-fn queue.
- **`2026-11-20-execute-the-deferred-hydrogen-round-2-fixes-*`** (iter-2715) —
  Hydrogen round-2, including its own T6 (slash commands duplicated across three
  operant-cli surfaces).
- **Owner-gated ledger rows** — RT-190 (memory bank), RT-280 (gateway
  resume/reset), RT-543 (two daemons), and the `[keybindings]` dispatch refactor.
- **The `[keybindings]` table question** — T5 of iter-2714 made `/keys` warn that
  the TUI ignores it. Whether the TUI should READ it is a dispatch refactor, still
  deliberately deferred.

## Cost

Four files of production text (three hint literals + one optional bucket row), one
new test module, one ledger row. Roughly one hour including the mutation proof and
the scoped verification. No schema, no config, no new dependency.