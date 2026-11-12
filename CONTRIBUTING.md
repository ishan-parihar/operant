# Contributing to operant

Thanks for looking at this. Here is what a contribution needs to be, and how to check
it before you open the pull request.

## The contract

1. **One thing per pull request.** A review is a reading, not a negotiation.
2. **The test comes with the change.** A behaviour change without a test is a claim, not
   a change.
3. **You are credited.** Every merged pull request is named in the release notes.
4. **The register is the record.** This project keeps open work in `TODO.md`, one row per
   item, with the verdict appended. If your change closes a row, say so in the pull
   request — and if it does not close one, say which row it belongs to.
5. **Nothing is deleted to make a row go away.** A row moves to done on evidence.

## Build and test

```bash
scripts/build.sh --release        # build
cargo test --workspace    # the tests
cargo fmt --all       # formatting
bash scripts/clippy-warning-gate.sh   # lints
```

Every one of these must pass before a pull request is opened. If a gate fails on a
change you did not make, say so in the pull request rather than fixing it silently in
the same diff.

## The shape of a good first issue

The issues labelled `good first issue` are real tasks with three things spelled out:

- **the reproduction** — the failing input or command,
- **the file to change** — where the fix belongs,
- **the test that must pass** — the check that proves it works.

If you pick one up and your understanding differs, open a pull request that says so:
the disagreement is worth more than a quiet fix.

## Where things live

- `AGENTS.md` — the project's own map: where the code is, and the laws this repository
  operates from. Read it before you change anything.
- `genome.toml` — the project's own order. A contribution has an owner to amend.
- `TODO.md` — the register of open work.

## Review

A maintainer reads the pull request, runs the gates, and responds. If a gate fails and
the fix is not obvious, you will be told what failed and where — not left guessing.

## Code of Conduct

This project follows the [Contributor Covenant](CODE_OF_CONDUCT.md). Report anything
that breaks it through GitHub's private vulnerability reporting on this repository.
