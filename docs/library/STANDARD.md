# The documentation standard — read this before writing any page

This is the contract every `docs/library/` page is held to. It exists because
"documented" is not a binary: a page can cite a module and still leave a reader
unable to answer the two questions that matter — *how does this work* and *how do
I use it*.

## The two questions every page answers

1. **How it works** — the mechanism, its data flow, the seams it crosses, and
   why it is built that way (the constraint or incident that shaped it).
2. **How to use it** — the surface a human or an agent actually touches: the
   config keys, the CLI commands, the tool names, the failure modes and what the
   operator sees when they fire.

A page that answers only (1) is a design note. A page that answers only (2) is a
man page. The library is both, because its readers are both.

## Required structure

```markdown
# <Element> — <the one-line claim>

Owns: <what question this page answers>
Not for: <which sibling page answers the adjacent question> ([sibling.md](sibling.md))

Source: `crates/.../module.rs` (what it holds), `crates/.../other.rs` (what it does)

## <The mechanism, in prose a newcomer follows>
## <Data flow / lifecycle — a numbered walk or a mermaid diagram>
## <The seams: what this crosses and what crosses it>
## Configuration — the keys, their defaults, and what each actually changes
## Surfaces — the CLI commands, tools, slash commands, or API a user touches
## Failure modes — what breaks, what the operator sees, what to do
## Intention — why it is this way, and what it deliberately is not
```

Not every section applies to every element. A pure-internal module may have no
Configuration section — but then it says so in one line, because an absent
section and an unconsidered one look identical to a reader.

## Rules

1. **Every factual claim is derived from the code, never copied from another
   doc** (RG-008). A count moves when the code moves; if you state a count,
   state it as of the commit you read.
2. **Every `crates/...` path you cite must exist** — `genome lint` E303 fails the
   build on a stale one.
3. **Config keys must be real.** Quote them from `crates/operant-core/src/config.rs`
   (or the `operant-config` schema) and give the default you read there. A key
   that does not exist is worse than no key.
4. **Tool names must be the registered names**, not the prose names. Check
   `crates/operant-core/src/tools/builtin.rs` and the family module.
5. **Name the failure mode, not the happy path only.** The library's value over
   a README is that it says what happens when the subprocess is missing, when the
   key is absent, when the provider 429s.
6. **Cross-link, do not duplicate.** If a mechanism is owned by another page,
   link it in `Not for:` and move on.
7. **No em dashes in new prose.** (Repo style; the existing pages predate it and
   are not being rewritten for it.)
8. **End with `## Intention`.** The mechanism exists for a reason; a reader who
   knows the reason can predict the next change.

## Depth target

A page that owns a subsystem of N modules should cite at least the modules a
reader must open to change it, and should be long enough that a competent engineer
who has never seen the code can make a correct small change after reading it.
Pages under ~120 lines that own more than three modules are under-documented;
pages over ~400 lines that own one module are over-explaining.

## The coverage gate

`genome lint` E311 fails when a source file is named by no library page. If you
write a page that covers a previously-undocumented module, its E311 row
disappears. If a module genuinely needs no page (a test fixture, a generated
file), add it to `ops.coverage_exempt` in `genome.toml` **with the reason** — an
exemption without a reason is a deferral with the paperwork missing.

## Backticks are a claim about a symbol

`scripts/verify-library.sh` reads every backticked snake_case name in a library
page HEADING and checks it against the tool registry, the guardrail name lists,
the alias table and the config-value table (all four derived from the code). So:

- **`file_read`** means the tool of that name. It had better exist.
- A *topic* that happens to look like one — `accessibility_tree` meaning the
  accessibility tree, `notification` meaning the notification subsystem,
  `transcription` meaning the capability — must be written plainly:
  *the accessibility tree*, *the notification subsystem*, *speech-to-text*.
  Backticking a topic is not just imprecise, it fails the build, because the
  check cannot tell a claim from a coincidence.

Same rule for config keys: a backticked `snake_case` name in a Configuration
section is a key that must exist in `crates/operant-core/src/config.rs`.

A `path:NNN` line citation is ADVISORY, not a claim: line numbers drift every
time the file changes, so a citation that was exact when written becomes wrong
months later and nothing catches it. Cite the module by path (which the E303 gate
checks exists) and the symbol by name; add `:NNN` only when the line number adds
reader value beyond the symbol, and expect it to rot.

## What the gate runs against your page

Five codes, and the first four are what the genome runs on every lint:

- **E303** — every `crates/...` and `scripts/...` path you cite must exist.
- **E301** — the page must be listed in `docs/INDEX.md`.
- **E311** — the modules your page names must be named by SOME library page; this
  is the reverse-coverage gate (every source unit is owned by something).
- **E020** — the page's own contract: its `Source:` header, its
  `Owns:` / `Not for:` boundary, its sibling links, and every tool name that
  appears in a heading (RG-023: a contract no gate runs is a contract nobody
  has).
- **E019** — a count you state must match the code (see above).

The commands that reproduce all five locally:

```
./scripts/genome lint
bash scripts/verify-library.sh
bash scripts/check-doc-claims.sh
```

A page that passes all five is one a reader can trust: it says what it owns, the
symbols it names resolve, and the numbers it states are true today.
