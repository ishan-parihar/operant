# Red team 6 + ponytail audit — the fix was correct and never reached the operator

**Date:** 2026-11-15
**Operator report:** "In the deployed binary, I am still unable to move the
cursor in the lines."
**Base:** `origin/main` at `01b26ae7` (my `ceee3c1c`).
**Verdict:** the code is right, the tests are right, and the binary the operator
runs was built from a tree that predates the fix by 90 commits. This is not a
code defect. It is a delivery defect, and the interesting part is why every
check I ran said "done".

---

## Finding 1 — the deployed binary does not contain the fix (ROOT CAUSE)

Measured, not inferred:

| | path | commit | `saturating_sub(4)` | `composer_text_width` |
|---|---|---|---|---|
| the fix | my `origin/main` | `ceee3c1c` | 0 | 1 |
| the operator's tree | `~/Documents/github/my-projects/agentic-harness/operant` | `61b17f4a` | **6** | **0** |

The operator's working tree is **90 commits behind and 8 ahead** of
`origin/main`, and `git merge-base --is-ancestor ceee3c1c HEAD` reports **NO**.
That tree still carries `area.width.saturating_sub(4)` in **six** places in
`key_handling.rs` — the exact arithmetic I fixed — and has no
`composer_text_width` at all.

So "the deployed binary is still broken" is literally true: that binary was
built from that tree. Every check I ran in the previous iteration (the fix, the
tests, the live tmux run, the md5 against the artifact I installed) was correct
**about a binary the operator does not run.**

### Why my provenance gate passed and still lied

The gate compares the artifact's mtime to the commit time, and the md5 of the
running process to the deployed path. Both held at 11:57. At **12:24** the
deployed binary was replaced by a build out of the operator's tree (different
md5, later mtime) — a legitimate build from a legitimate tree, so nothing
looked wrong. My gate answers "is the deployed binary the one I just built",
which is a *different* question from "does the deployed binary contain the
change". The second question needs a **content** check, and that is the gap.

A `strings`/`nm` probe for the fix's own identifiers would have caught it in
seconds. I ran those probes at 07:26 and they returned 0, and I read the 0 as
"stripped release binary" rather than as "this is not my build". That was the
miss: **a zero from a symbol probe on a stripped binary is not evidence of
absence unless the symbol is known to be present in a known-good build.** The
control run I did not do was to probe my own freshly built artifact and confirm
the string is absent there too (it is — it is a test-only literal). The right
probe is a *runtime* one.

## Finding 2 — the ponytail audit: three checks that cannot fail

| check I ran | what it actually proves | what it does not |
|---|---|---|
| `cargo test -p operant-cli` | the source in the tree I edited is correct | nothing about the binary the operator runs |
| `md5sum /proc/PID/exe` vs the deploy path | the daemon exec'd what is on disk right now | that it is the build containing the change |
| `ls -l target/release/operant` mtime >= commit | this tree produced an artifact after the commit | that this tree has the commit |
| live tmux walk on my own debug build | the walk works in my tree | that the operator's tree walks |

None of them is wrong. All four are **indirect**: each measures a proxy one
step removed from the operator's actual experience. The only check that
answers the operator's question is *drive the binary the operator runs, on the
operator's tree, and watch the caret.* I did that at 07:34 — on the deployed
binary — and it walked correctly, because at that moment the deployed binary
**was** my build. It was replaced 10 minutes later.

## Finding 3 — the one-line delivery invariant that was missing

The R&D protocol says "deploy after every push". It does not say *deploy from a
tree that contains the commit*. Those differ the moment more than one tree
exists on the box, and there are currently **eleven** worktrees plus the
operator's tree.

Proposed invariant, and it is cheap: before installing, assert that the artifact
contains the change, by a probe that is known to be present in the artifact and
absent from a pre-change build. For a width change that is hard (no new
literal); for this fix the honest probe is behavioral — `operant tui debug
simulate` with a key sequence, not a symbol.

## What I did NOT do, deliberately

I did not "fix" the single-row case again. Round 2 of this session tried
exactly that — treat "the composer holds one row" as a first-class answer so Up
would not recall — and it broke **three** tests that encode the rule the
operator asked for in the first place: "the up/down arrow navigates the cursor
within the lines, **until it touches the top/bottom**." A single-row draft is
already at both edges, so Up meaning history is the specified behaviour, not a
bug. Round 2 was reverted in full (handler, enum, export, tests) and this
report records it so the next agent does not re-derive it.

That is the whole of the answer to the operator's sentence: there is nothing
left to fix in the walk. The walk is right. The binary is old.

## The fix the operator needs

In their tree:

```
git -C ~/Documents/github/my-projects/agentic-harness/operant fetch origin
git -C ~/Documents/github/my-projects/agentic-harness/operant pull --rebase origin main
scripts/build.sh --release
# then install to the PATH-resolved path the gateway's ExecStart names:
cp target/release/operant ~/.local/bin/operant.new && mv ~/.local/bin/operant.new ~/.local/bin/operant
systemctl --user restart operant-gateway
```

`git merge-tree --write-tree` (read-only, nothing written) says the rebase will
conflict in **four** files: `commands.rs`, `key_handling.rs`, `providers.rs`,
`tests.rs` — all four from their 8 unpushed commits, none of them the composer
width. I did not run the pull: it is the operator's tree with their uncommitted
work, and resolving their conflicts is theirs to make.

## Gates for this iteration (docs-only)

No code change. `docs/audit/2026-06-19-BUGS.md` gains RT-580.