# LAW — the rules a proposal must survive

These are the subject's binding rules, quoted verbatim from `AGENTS.md`. A recommendation
that violates one is not admissible, however good it looks.

---

## The Iron Rule

> **ALWAYS PUSH COMMITS AFTER EACH ITERATION.**
>
> After every iteration — without exception — you must:
>   1. commit the change locally, and
>   2. push the commit to `origin/main`.
>
> The user pulls from `origin/main` to test on their local system. **A change that lives
> only in your working tree is not done.** If `git log` on `origin/main` does not show
> your iteration, you have not delivered it.

**Consequence for this round:** a recommendation whose first step is "do not ship this
yet" is admissible only if it names what would be shipped instead. Deferral with no
successor is not an option the law allows.

## Parallel-agent discipline (the rules that exist because of a real incident)

> 1. **The iteration-label authority is `origin/main`, hand-computed.**
> 2. **The main working tree may carry a peer's uncommitted WIP and even their local-only
>    WIP commits.** NEVER `git reset --hard`, `git clean`, `checkout` over it, or
>    update-ref its branch without preserving first.
> 3. **Never `git add -A`.** Stage explicit paths.
> 7. **If the tip is red from a peer's in-flight subsystem, don't fix it and don't block
>    on it.** Prove your delta adds zero (error-count before/after on the untouched base
>    vs patched), say so in the commit body, push.
> 8. **A slice is yours or nobody's.** Never amend, rebase, or delete a
>    branch/worktree you did not create.
> 12. **A shared worktree's HEAD is DETACHED and SHARED — a peer's `reset` can delete
>    YOUR commit, so push the moment you commit and know how to get it back.**

**Consequence for this round:** a recommendation that requires a wide, sweeping edit to
contended files (the gateway, the TUI) has a coordination cost the law prices. Say so.

## Design preferences — DO NOT CHANGE

> These are the user's intentional design choices. Do not replace, remove, or "improve"
> them.

- **Voice / TTS:** Phonon (`phonon-31m-en`, GGUF q8_0). "Do NOT switch to Edge TTS,
  OpenAI TTS, or any other provider as the default."
- **Memory:** memory-wire is the ONLY engine. All legacy providers were removed.
- **Browser & Web Tools:** sourcehound is the default. "Do NOT switch the default away
  from sourcehound."
- **Platform Adapters (Gateway):** 7 platforms. "Do NOT re-add purged platforms without
  a real adapter implementation."
- **Context Management:** tiered eviction + decay curve, ported from
  `cortexkit/magic-context`.

**Consequence for this round:** R3's option (a) — pointing operant at the memory-wire
serve over HTTP — **trades against a design preference**, because the HTTP surface does
not expose `recall_with_weights` / `retain_doc`, which are the in-process advantage. Any
recommendation for (a) must say what it gives up.

## The Rust Code Standards (baseline for every new/edited Rust)

> - Prefer `&T` over `.clone()`; clone only when ownership genuinely transfers.
> - Fallible operations return `Result<T, E>`; never `panic!` in production paths.
> - **Production code must not call `.unwrap()` / `.expect()`.** Justified invariant
>   sites carry `#[expect(clippy::unwrap_used, reason = "...")]`.
> - Degrade, don't die, at subsystem seams: memory/provider/tool failures must turn into
>   a logged miss or a helpful tool error, never a dead turn.
> - Every fix ships a regression test that fails without the fix.
> - Never weaken the gate (`--update` exists only to prune warnings you FIXED, never to
>   hide new ones).

**Consequence for this round:** a recommendation to silence a finding by allowlisting it
**violates the last bullet** and is not admissible. A recommendation to *narrow* an
allowlist is.

## Compile strategy (the budget law)

> - **Never** run `cargo check --workspace` mid-iteration. A full workspace compile
>   takes 10+ minutes on a 2-CPU box.
> - **Always** scope cargo commands to the crate being edited.
> - Reserve `--workspace` for the **final** verification step right before push.

**Consequence for this round:** a recommendation whose verification step is "run the
workspace suite" carries a real compute cost. Say so, and scope it.

## The documentation genome's five laws

> 1. **One owner per question.** Two documents may not answer the same thing.
> 2. **Every document has a mutability class.**
> 3. **Nothing is deleted.** Superseded → `genome archive`.
> 4. **Statuses move through the vocabulary** (`open` `in-progress` `done` `rejected`
>    `blocked`); terminal states name their evidence.
> 5. **Parallel agents claim before they work.**

**Consequence for this round:** closing RT-543 / RT-280 / RT-190 by **deleting** the
surface is admissible only as a *ruled* status with evidence, or via
`genome archive` for documents. A code deletion still ships as a normal iteration, but
the ledger row's terminal state must name its evidence (law 4).

## The deployment law

> **After EVERY iteration that touches code (not docs-only):** build the release binary
> and install it so the user can immediately test the change. A commit that isn't
> deployable is not done.

**Consequence for this round:** a recommendation that changes runtime behaviour must name
its deploy step. A docs-only recommendation is exempt and should say so.