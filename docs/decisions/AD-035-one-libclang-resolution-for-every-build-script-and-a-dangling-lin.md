---
ID: "AD-035"
Title: "One libclang resolution for every build script, and a dangling link is not a libclang"
Status: "Active"
Date: "2026-10-19"
Description: "dev-env.sh, check.sh and provision-build-deps.sh each resolved libclang differently, and the one dev-env.sh used fell through to a directory that does not exist; a shared resolver now owns the choice, and it rejects a symlink whose target the box removed"
---

# AD-035: One libclang resolution for every build script, and a dangling link is not a libclang

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

The build failed on this box with:

```
thread 'main' panicked at .../bindgen-0.69.5/lib.rs:622:31:
Unable to find libclang: "couldn't find any valid shared libraries matching:
['libclang.so', 'libclang-*.so', 'libclang.so.*', 'libclang-*.so.*'] ...
(invalid: [.../local/libclang/libclang.so: No such file or directory (os error 2)])
```

The path it complained about was `local/libclang/libclang.so`, a symlink to
`/usr/lib/libclang.so.22.1`. The box upgraded libclang to 23 and the target
went away, so the link was left dangling. The build had been failing this way
since the upgrade, and the error named neither the cause nor the fix.

Reading the three build scripts found three different resolutions, and none
of them was correct:

| script | what it did | what went wrong |
|---|---|---|
| `scripts/dev-env.sh` | preferred `local/libclang/libclang.so` if the path existed, else fell through to `local/libclang_extract/usr/lib/x86_64-linux-gnu` | `-e` on a dangling symlink is false, so it skipped the provisioned link; the fallback directory exists only after the Debian `.deb` path ran, so it did not exist either. It only printed a WARN, and the build died two minutes later inside bindgen. |
| `scripts/check.sh` | `libclang_extract` → `/usr/lib` → `/usr/lib/llvm21/lib` | the LLVM version was hardcoded, so a box with llvm22 or llvm23 fell through to no `LIBCLANG_PATH` at all |
| `scripts/provision-build-deps.sh` | globbed `/usr/lib`, `/usr/lib/libclang.so.*.*`, `/usr/lib/*/libclang.so.*.*` | the third pattern is one level too shallow for `/usr/lib/llvm-N/lib/`, and `ln -sf` on an existing link to a still-valid target was fine, but a link to a removed target was left as it was |

So `check.sh` found `/usr/lib` and the build worked; `dev-env.sh` found
nothing and the build died. Two scripts, one box, opposite outcomes, and
which one ran decided whether the build worked.

Two more facts about bindgen's contract, both of which the old code got
wrong:

- bindgen globs for `libclang.so`, `libclang-*.so`, `libclang.so.*` and
  `libclang-*.so.*`. A directory holding only `libclang.so.N.N` satisfies the
  last pattern, but a directory holding a name outside that set does not.
- "the path exists" is not the test. A symlink to a removed target exists as
  a directory entry and does not exist as a file, and it is the file that
  bindgen has to dlopen.

## Decision

One resolver, `scripts/libclang-resolve.sh`, sourced by all three scripts.
It owns the choice and the test, and it exports nothing of its own, so
sourcing it cannot change a caller's shell state.

**The test is "a globbable name in this directory resolves to a real file".**
`_libclang_dir_has_lib` implements exactly that: it walks bindgen's four
globs and requires `-e` on the match, which follows the symlink. A dangling
link is rejected.

**The search order is:**

1. an explicit `LIBCLANG_PATH`, when it actually holds a libclang
2. the provisioned link directory, `LOCAL_DIR/libclang`
3. the system directories, including the `/usr/lib/llvm-N/lib` layout
4. the extracted Debian `.deb` directory, the last-resort fallback

An explicit `LIBCLANG_PATH` that holds nothing is ignored, and the search
continues. A caller who set a bad path by mistake is not obeyed into a
broken build; the resolver's own choice is used instead, and the script
prints the path it chose.

**A missing libclang is a startup error, not a warning.** `dev-env.sh` and
`check.sh` now exit 1 with the path they could not find and the command that
fixes it. The old behaviour — a warning, then a bindgen panic two minutes
later — cost a full build cycle to diagnose and named neither the cause nor
the fix.

**The provisioner repairs a dangling link.** It looks up the target
separately from the link, so a link whose target the box removed is replaced
rather than left in place, and it says so.

## Evidence

- `bash scripts/tests/libclang_resolve_test.sh`: PASS. The test pins the
  seven properties that matter, against a fake filesystem: a versioned-only
  layout is accepted, a dangling symlink is rejected, an empty directory is
  rejected, an explicit valid `LIBCLANG_PATH` wins, an explicit invalid one
  is ignored and the search continues, the provisioned link directory wins
  over the system search, and `dev-env.sh` agrees with the resolver.
- `bash scripts/provision-build-deps.sh` on this box: `[provision] repairing
  a dangling libclang link` → `using system libclang: /usr/lib/libclang.so`.
  The link now resolves to `/usr/lib/libclang.so.23.1.3`.
- `cargo build -p operant-cli --bin operant`: `Finished dev profile` in
  1m 43s, with the previous build failing in `espeak-rs-sys`'s bindgen step.
- `bash scripts/tests/build_sh_test.sh`: 5 of 5 assertions pass, so the
  resolver did not disturb the memory-wire sync ordering.

## Consequences

- A box with libclang only under `/usr/lib/llvm-N/lib` builds. Before, the
  provisioner downloaded a Debian `.deb` it did not need, and `check.sh`
  found nothing unless N was exactly 21.
- A libclang upgrade on the box no longer breaks the build. The dangling
  link is repaired by the next provisioning run, and the resolver rejects it
  in the meantime rather than trusting it.
- The three scripts cannot drift apart again: they all call the same
  function, and the test asserts they agree.

## Open

- The Debian `.deb` fallback still pins libclang 19. It is a last resort for
  a box with no system libclang at all, and it is not exercised here.
