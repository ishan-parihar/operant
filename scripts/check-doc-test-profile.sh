#!/usr/bin/env bash
# The cfg(test) rustdoc profile — the second doc profile RG-013 prescribes.
#
# WHY THIS EXISTS. The production doc gate (`./scripts/check.sh doc`) runs
# `cargo doc --workspace --no-deps --all-features --document-private-items`
# with `cfg(test)` OFF, because a normal doc build never turns it on. A
# `#[cfg(test)]` item is therefore not in the AST rustdoc sees, and a broken
# intra-doc link inside test-only documentation is invisible to it — the exact
# defect RG-013 records (`RenderStateTestGuard` linking to a `render_state_test_lock`
# that was never ported, with the gate green its whole life).
#
# WHAT WAS MEASURED before this was written (iter-1176). Three candidate
# mechanisms were tried and each failed differently; all three are recorded
# because each one LOOKED like it worked:
#
#   (i)   `--cfg test` as a CARGO ARGUMENT is rejected. `cargo doc ...
#         --cfg test` (and `cargo rustdoc ... -- --cfg test`) both fail with
#         `error: unexpected argument '--cfg' found` — cargo 1.98.0 parses the
#         flag itself instead of forwarding it. The flag must ride RUSTDOCFLAGS.
#   (ii)  RUSTDOCFLAGS works, but `--no-deps` alone makes the profile
#         UNBUILDABLE for any crate whose test code imports a dev-dependency:
#         `cargo doc --no-deps` does not resolve `[dev-dependencies]`, so
#         rustdoc sees `use serial_test;` with no crate to name. operant-core
#         reports `unresolved import mockito` plus five `serial_test` errors;
#         operant-cli one.
#   (iii) Dropping `--no-deps` and letting cargo document the dependencies is
#         WORSE, and silently so: RUSTDOCFLAGS applies to every crate cargo
#         documents, so the DEPENDENCIES' own cfg(test) profiles run too — and
#         memchr 2.8.3's `define_memchr_quickcheck!` imports `quickcheck`, which
#         is memchr's DEV-dependency and absent here. cargo dies with
#         `error: could not document \`memchr\`` and NEVER REACHES OUR CRATE.
#         A gate reading only cargo's summary lines sees neither a failure nor
#         a success and defaults to GREEN. That is exactly what the first
#         version of this script did.
#
# SO THE MECHANISM IS THREE STEPS PER CRATE:
#   1. `cargo test -p <crate> --no-run` — compiles the crate's TEST profile,
#      which resolves `[dev-dependencies]` and writes their artifacts.
#      NOTE: `--lib --bins` together is WRONG here: a crate with no library
#      target (operant-cli is a binary) answers `error: no library targets
#      found in package` and the gate skipped it as a broken test profile when
#      its profile compiles fine. Plain `--no-run` selects every target.
#   2. `cargo doc -p <crate> --no-deps` — documents ONLY this crate, so no
#      third-party cfg(test) profile runs and no dependency can mask ours.
#   3. `--extern <dep>=<path>` for each dev-dependency on RUSTDOCFLAGS, so
#      rustdoc can resolve the test-only imports. Those paths are taken from
#      cargo's OWN `-v` output (see the long note at the externs block below),
#      never guessed from a directory listing.
#
# A non-zero exit with NO diagnostic naming THIS crate is reported as BROKEN,
# never as a pass: "I could not check" is not "it is clean". And every crate
# that is documented prints its own line, so a GREEN with no `OK` lines beneath
# it is visibly meaningless rather than silently so.
#
# Usage:
#   ./scripts/check.sh doc-test                # report only, always exit 0
#   ./scripts/check.sh doc-test --gate         # exit non-zero on any failure
set -uo pipefail

DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
. "$DIR/dev-env.sh"
cd "$DIR/.."

# CANCEL dev-env.sh's `set -e` (its line 13). Sourcing it imports errexit into
# THIS script, and errexit is exactly what a gate must not honour: the whole
# glyph of the gate is that `cargo doc` returns 101 on a broken cfg(test) doc
# link and the gate REPORTS that crate and carries on to the next ten. Under
# errexit the very first 101 aborts the script — which is what happened, and it
# looked like a broken loop: the header printed the crate list, no per-crate
# line ever appeared, and the process exited 101. Same shape as iter-1159 (one
# copy of the build environment, and the copy that imports shell options by
# accident). `set +e` restores the documented `set -uo pipefail` above.
set +e

MODE="${1:-}"
if [ "$MODE" = "--gate" ] || [ "$MODE" = "gate" ]; then GATE=1; else GATE=0; fi

# Read the [workspace] members list ONLY — never a `path = "crates/.."` line
# elsewhere in the manifest (the first version of this regex scanned the whole
# file and returned every crate TWICE, so each crate was documented twice).
#
# The per-crate TARGETS come from each crate's own manifest. The obvious
# spelling `cargo doc --lib --bin` is wrong: `--bin` takes an argument, so a
# bare `--bin` fails with `error: "--bin" takes one argument.` and all ten
# lib-only crates fail on it instead of being documented — and because the
# first version's filter only looked for the crate's own name, that error was
# DISCARDED and every crate printed OK. `--bins` needs no argument, which is
# why it is used here.
CRATES=$(python3 - <<'PY'
import pathlib, re
src = pathlib.Path("Cargo.toml").read_text()
members = src[src.index("[workspace]"):]
j = members.index("members = [")
k = members.index("]", j)
for n in sorted(set(re.findall(r'"crates/([^"]+)"', members[j:k]))):
    man = pathlib.Path("crates", n, "Cargo.toml").read_text()
    flags = []
    if pathlib.Path("crates", n, "src", "lib.rs").exists():
        flags.append("--lib")
    if re.search(r'^\[\[bin\]\]', man, re.M):
        flags.append("--bins")
    if flags:
        print(n, " ".join(flags))
PY
)

say() { printf '%s\n' "$*"; }

fail_total=0
skipped=0
say "== cfg(test) rustdoc profile (RG-013) =="
say "crates: $(printf '%s' "$CRATES" | tr '\n' ' ')"
say ""

# An ARRAY, not `while read` and not a word-split `for`.
#
#   * `while read` reads from stdin, which this script's own
#     `cargo test ... </dev/null` and the `CRATES=$(...)` substitution have
#     already consumed — so the loop ran EXACTLY ONCE and fell through to the
#     GREEN banner having documented no crate at all.
#   * `for line in $CRATES` does not survive the crate/flags split either:
#     `for` word-splits on whitespace, so a line becomes TWO words and
#     `${line%% *}` / `${line#* }` then yield `crate=operant-api` with
#     `flags=operant-api` — the crate name as its own target. cargo is handed
#     `cargo doc -p operant-api operant-api`, it fails, and the classifier
#     reported that failure as OK.
#
# `mapfile` keeps each line intact, so `crate` and `flags` are two fields of
# one array element.
mapfile -t CRATE_LINES <<<"$CRATES"

for line in "${CRATE_LINES[@]}"; do
    crate=${line%% *}
    flags=${line#* }
    [ -n "$crate" ] || continue

    # STEP 1 — compile the crate's own test profile first (measurement (ii)).
    # A test profile that does not COMPILE is a defect of its own, and it is
    # the one `cargo test` owns; report and move on so the rest of the
    # workspace is still checked.
    if ! cargo test -p "$crate" --no-run >/dev/null 2>&1; then
        say "SKIP  $crate — its test profile does not compile; not documented (cargo test owns that failure)"
        skipped=$((skipped + 1))
        continue
    fi

    # STEP 3's inputs — every dev-dependency THIS crate's test build
    # actually resolved, as `--extern name=path` for RUSTDOCFLAGS.
    #
    # Two wrong mechanisms were tried before the right one, and both failed in
    # the direction that looks like broken code rather than a broken gate:
    #
    # (a) glob `target/debug/deps/lib<name>-*.rmeta` and take the FIRST hit.
    #     `tempfile` has 33 candidates there because every feature combination
    #     of every crate that depends on it leaves its own copy; alphabetically
    #     first is a DIFFERENT build from this crate's, and rustdoc rejects it
    #     with `error[E0464]: multiple candidates for 'rmeta' dependency`.
    #
    # (b) read the `deps` array of the crate's `*.json` fingerprint. It is
    #     `[hash:int, name:str, public:bool, extra]` — there is NO path in it,
    #     and that int hash is the dep's metadata hash, not the filename
    #     suffix, so it does not even locate a file. Every dep was filtered
    #     out, the gate emitted no externs, and it then reported all four
    #     crates as BROKEN.
    #
    # The right one: make cargo itself reveal the resolution. `cargo test -p
    # <crate> --lib --bins --no-run -v` compiles the crate's test profile and
    # prints the exact rustc invocation, whose `--extern name=path` pairs ARE
    # the dev-dependency resolution for this crate. Extracting them and
    # reusing them for rustdoc means rustdoc sees the same crates rustc saw —
    # no guessing, no staleness, no candidate ambiguity.
    #
    # A fresh crate is CACHED, and on a cache hit cargo prints no rustc line
    # at all, so the crate's source file is touched to force exactly one
    # rebuild of its own test profile.
    #
    # WHICH file, though, matters: `touch lib.rs` blindly CREATES an empty
    # `crates/operant-cli/src/lib.rs` (operant-cli is a binary with no lib
    # target), and an empty lib.rs makes cargo conclude the crate HAS a
    # library target — changing the `--lib`/`--bins` target set the gate reads
    # from each manifest. That is a side effect that outlives the script: the
    # file stays on disk and the next `cargo doc`/`cargo build` run documents a
    # different target set. So touch a file that is ALREADY there: lib.rs when
    # it exists, main.rs when the crate has a bin, and neither when it does
    # not. Never create.
    #
    # This is not a source edit (the gate writes no content), and cargo
    # regenerates the fingerprint afterwards, so the build cache stays coherent.
    if [ -f "crates/$crate/src/lib.rs" ]; then
        touch "crates/$crate/src/lib.rs"
    elif [ -f "crates/$crate/src/main.rs" ]; then
        touch "crates/$crate/src/main.rs"
    fi
    externs=$(cargo test -p "$crate" --no-run -v 2>&1 \
        | grep -oE -- '--extern [a-z_0-9]+=[^ ]+' \
        | grep -vE '^--extern operant_' \
        | awk -F'=' '!seen[$1]++' \
        | tr '\n' ' ')


    # STEP 2+3 — document only this crate, with the dev-deps named.
    out=$(RUSTDOCFLAGS="${RUSTDOCFLAGS:--Dwarnings} --cfg test $externs" \
          timeout 900 cargo doc -p "$crate" $flags --no-deps \
          --document-private-items 2>&1)
    rc=$?

    if [ "$rc" -eq 0 ]; then
        say "OK    $crate"
        continue
    fi

    # (a) cargo rejected the invocation. The gate cannot certify anything.
    if printf '%s\n' "$out" | grep -qE 'unexpected argument|unrecognized subcommand'; then
        say "BROKEN $crate — cargo rejected the invocation; this is the gate's own defect, not the code's:"
        printf '%s\n' "$out" | head -10
        fail_total=$((fail_total + 1))
        continue
    fi

    # (b) A diagnostic naming a path in THIS crate is ours: a broken
    # cfg(test) doc link, an unresolved item, a doc comment rustdoc rejects.
    # This is the class RG-013 records, and it is what the gate is for.
    #
    # Two things are printed, because the first version printed only paths and
    # the probe test then could not find the defect's NAME: a run with a planted
    # broken link in `operant-core` printed 30 `--> crates/...` lines of other
    # diagnostics and cut the probe's own line out through `head -30`. An
    # operator reading that output cannot tell what broke. So the `error:` /
    # `warning:` headline lines come first (they carry the words), then the
    # paths of the diagnostics under this crate.
    ours=$(printf '%s\n' "$out" \
        | grep -E '^(error|warning)(\[[^]]*\])?:|^crates/' \
        | head -40)
    if [ -n "$ours" ]; then
        say "FAIL  $crate (rc=$rc) — THIS crate's cfg(test) profile is broken:"
        printf '%s\n' "$ours" | sed 's/^/      /'
        fail_total=$((fail_total + 1))
        continue
    fi

    # (c) No diagnostics of ours and a non-zero exit. The gate cannot certify
    # this crate, so it must not be reported as a pass.
    say "BROKEN $crate (rc=$rc) — cargo exited non-zero with NO diagnostic naming this crate, so its docs were never built. This is NOT a pass:"
    printf '%s\n' "$out" | tail -25 | sed 's/^/      /'
    fail_total=$((fail_total + 1))
    continue
done

say ""
if [ "$fail_total" -eq 0 ]; then
    if [ "$skipped" -gt 0 ]; then
        say "cfg(test) doc profile: GREEN on every crate it documented ($skipped crate(s) SKIPPED for a test profile that does not compile — cargo test owns those)"
    else
        say "cfg(test) doc profile: GREEN — every crate's test profile compiles and no cfg(test) doc link is broken"
    fi
    exit 0
fi
say "cfg(test) doc profile: $fail_total crate(s) failing, $skipped skipped"
if [ "$GATE" = "1" ]; then exit 1; fi
exit 0
