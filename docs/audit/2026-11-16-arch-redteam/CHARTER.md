# Architecture red-team + ponytail audit (2026-11-16)

AUDITED REVISION: origin/main @ 2d431ac0 (pinned; every finding must cite this)
TREE STATE: the shared working tree is DIRTY with a peer session's 19 modified
files. It is NOT the system under audit. Read code only via:
  git show origin/main:<path>
  git grep -n 'PAT' origin/main -- <path>
Never `cat`/`sed -n` a working-tree file. Never edit, stage, commit, or run cargo.
Never run a git write command. A finding measured against the dirty tree is void.

## The three questions every worker answers

1. RED-TEAM: what is broken? A real failing case with input + wrong output.
   Trace it end to end or label it ASSERTED. A guess is not a finding.
2. PONYTAIL: what is dead or redundant? Say what to DELETE, not rename.
   Prove it: zero non-test references. Beware cross-crate name collisions —
   a bare `foo::bar` grep hits `operant_core::other::bar` in a different crate.
3. RUST DISCIPLINE: read
   /home/ishanp/.agents/skills/rust-best-practices/references/chapter_01.md (idioms,
   borrowing), chapter_03.md (perf), chapter_04.md (errors), chapter_06.md
   (generics/dispatch), chapter_09.md (Send/Sync/pointers) FIRST, then flag:
   `.clone()` in loops, `String`/`Vec<T>` params that should be `&str`/`&[T]`,
   `.unwrap()`/`.expect()` in production (tests exempt), `panic!`, `anyhow` in
   library crates (must be typed errors), needless `.collect()`,
   `large_enum_variant`, `#[allow]` where `#[expect]` + reason belongs.

## Ranking
CRITICAL (data loss / security / wedge) > HIGH (wrong behavior a user hits) >
MEDIUM (perf, clarity) > LOW (style). Never manufacture a finding to fill a
section. An empty section is a correct section.

## Deliverable
One markdown file per worker at docs/audit/2026-11-16-arch-redteam/<your-id>.md
with: VERIFIED / ASSERTED / LEAN CUTS / RUST DISCIPLINE / COVERAGE.
