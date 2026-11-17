# knowledge-base-red-team-gates-vs-reality

Dated-frozen: audited `2026-11-17`; findings mark verified vs asserted.

## Scope

A red-team audit of the knowledge-base gates this session built, not of the
documentation. The question was narrow and uncomfortable: **do E311, E019 and
E020 actually check what their names and docstrings claim?** Every finding
below was found by attacking my own gate, and every one is a case of a gate
that reported clean while the thing it guarded was broken.

Companion records: RG-022 (verification pass over new docs must be independent
of the writer) and RG-023 (doc-claim gates must derive from the code).

## Findings

- **[verified] E311's exemptions were unreadable, so their claims were
  unverifiable.** All 18 rows in `ops.coverage_exempt` read "leaf; owned by
  docs/library/X.md", but the reason was a TOML *trailing comment*, not part of
  the string. `genome.toml` admitted it in prose: "the reason is a comment on
  the row, not text inside the string". No gate could read the claim, so "was
  checked to really discuss the family" was itself unchecked. Fixed iter-2555:
  rows are now `"<path> -> <reason>"` with the reason inside the string, and
  E311 checks that the named page really contains the unit.

- **[verified] Once readable, 5 of 18 exemption claims were false.**
  `memory_traits.rs -> memory.md`, `peripherals_traits.rs -> tools.md`,
  `runtime_traits.rs -> agent-loop.md`, `cmd_continue.rs -> sessions.md`,
  `checks_integrations.rs -> operability.md` all named owner pages that never
  mentioned the unit. A further 13 named the wrong owner: every `cmd_*.rs`
  wrapper pointed at the subsystem page while the command itself is documented
  in `cli-surface.md`. All five false owners were made true by adding the
  missing prose to the claimed page; the 13 were repointed. No exemption was
  invented to silence the gate.

- **[verified] E311 required only a bare stem, so 267 of 926 units passed by
  coincidence.** The check was `name in body`. Measured on 2026-11-17: 189
  units were cited by full path, 470 by module token, and 267 only because
  their bare stem is an English word that appears somewhere in a library page.
  `space.rs` passed because 19 pages contain "space". `parse.rs` passed
  because "parse" appears in prose. Fixed iter-2556: a citation is now the
  full repo path or the module token as its own word, word-boundary matched.

- **[verified] The tightened gate exposed 151 genuinely uncited units**,
  62,787 lines of source, including the 2,214-line `gateway/telegram.rs`,
  `cronjobs/db.rs`, `agent/learning_graph.rs`, `agent/compress.rs`,
  `agent/pooled_client.rs`, `agent/message_safety.rs`, and 73 TUI modules.
  The old gate reported zero. This is the real documentation debt, and it is
  being paid down with citations rather than exemptions.

- **[verified] 12 library pages answered "how it works" and never "how to use
  it".** `browser`, `config`, `context-injection`, `harness`, `observability`,
  `operability`, `org`, `persistence`, `providers`, `sub-agents`,
  `background-review` and `tui-debugging` had mechanism sections but no
  operator-facing one. That is precisely the failure `docs/library/STANDARD.md`
  was written to prevent, in pages written after the standard existed.

- **[verified] 5 source comments cite deleted documents.** `peripherals_traits.rs`
  -> `docs/hardware-peripherals-design.md`; `operant-os/src/lib.rs` ->
  `docs/design/2026-10-14-holon-operant-boundary.md`; `cronjobs/scheduler.rs` ->
  `docs/plan-2026-10-07-two-tier-memory-hybrid.md`; `org/context_injection.rs` ->
  `docs/plan-2026-10-08-dm-feed-context-injection.md`; `binary_updater.rs` ->
  `docs/skills/smuggled.md`. None was archived, so none is findable. A reader
  who follows the pointer hits a dead end. (Two further hits,
  `docs/book/src/contributing/privacy.md` and `docs/a/readme.md`, are an
  upstream privacy stance and a test fixture respectively, not ours.)

- **[verified, partial overlap] E303 and E020 both catch a missing library
  path, but neither subsumes the other.** Mutation-tested: breaking a path in
  `docs/library/gateway.md` fires both; breaking one in `docs/INDEX.md` fires
  only E303 (E020 scans `docs/library/` only), while E020 uniquely covers
  headers, sibling links, TOC anchors and tool-name headings. Not redundant.

## What this audit is really about

Every finding has the same shape: **a gate that was satisfied while the thing
it guarded was not.** E311 said 0 uncovered files and was wrong by 151. Its
exemptions said "owned by X" and were wrong by 18. The failure was never a
missing check; it was a check whose notion of "covered" was weaker than the
word.

The durable rule: a gate's claim must be *checkable by the gate itself*. An
exemption reason in a comment, a bare-substring citation, a "was reviewed"
note with no artifact behind it. Each of these is paperwork that reads as
verification and is not.

## Intention

Written after the audit so the next agent does not re-derive it. The two gate
fixes (iter-2555, iter-2556) are the durable output; the citations and usage
sections are debt repayment that follows.