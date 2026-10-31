---
ID: "RG-017"
Title: "a green test summary can be served from cache while the crate is red"
Status: "Active"
Date: "2026-10-31"
Description: "a green test summary can be served from cache while the crate is red"
Severity: "High"
---

# RG-017: a green test summary can be served from cache while the crate is red

Immutable once numbered: a correction is a new record that supersedes this one.

## Symptom

An agent runs `cargo test -p <crate> --bin <bin> <filter>`, sees `test result: ok. 5 passed`,
and reports the crate green and its change verified. The crate is in fact RED: a peer's
dirty WIP in the same workspace has an E0382 that makes every dependent crate fail to
compile. Cargo serves the previously-built test binary, which still links and still
prints its old summary, so nothing fails loudly.

Observed 2026-10-31 (iter-1540): a 10-run mutation-proof loop produced nine clean
`5 passed` summaries and one `4 passed`. The `4 passed` was not a flaky test — it was the
one run where a concurrent peer build forced a recompile, hit the peer's E0382, and cargo
printed a partial summary that the agent's `grep` read as a pass count. The agent had
already published `10/10 green` twice before catching it.

The inverse trap is the same mechanism: `cargo check` returning `Finished in 0.24s` is
cargo reading fingerprints, not compiling. Two of the agent's "green" results that morning
were that.

## Root cause

`cargo test` reports on a binary it did not rebuild. When the source changed but the
fingerprint cache still considers the test binary fresh — or when the build is genuinely
blocked by another process holding the build-directory lock ("Blocking waiting for file
lock") — cargo can emit a summary for a binary that does not reflect the current tree.

Three reasons this is hard to notice:

1. A dirty peer file only becomes visible when something forces a recompile. `touch`ing your
   own file, a peer's build, or contention on the shared `target/` are the three triggers,
   and they fire irregularly — which is why the anomaly appeared in 1 of ~45 runs.
2. The failure is not a test failure. It is `error: could not compile` with a nonzero exit,
   so an agent parsing stdout for `^test result:` finds either nothing or a stale line.
3. The shared `target/` in fleet work means one agent's red file corrupts every other
   agent's measurements, in both directions.

## Law

Never report a test result you did not watch being computed. Before believing any
`test result:` line, prove the compile was real:

- `cargo test` that finishes in under ~2s did not compile. Force one:
  `touch <a file in the crate you changed>` then re-run, and check the elapsed time and
  the `Compiling`/`rustc` lines are present.
- Capture the full output, not a `grep` of its tail. A compile failure exits nonzero and
  prints `error: could not compile`; an agent that greps only for the summary line will
  miss it and report the previous count.
- Assert on the exit code as well as the text. `echo "exit=$?"` after every measurement.
- In fleet work, expect a peer's dirty file to be the cause. Distinguish "my change is
  broken" from "the shared tree is red" before reporting either, and disclose the peer's
  file rather than fixing it (fleet rule 7).

A mutation proof is only as good as its negative control: run the mutated build AND the
restored build under identical forcing conditions, or the comparison is meaningless.

## Detection

- Elapsed time: a sub-2s `cargo test` over a crate that changed is cached. Say so.
- `Compiling <crate>` in the output: absent means no rebuild happened.
- Exit code: nonzero with no `test result:` line means a compile failure the grep hid.
- `Blocking waiting for file lock on build directory`: another agent owns the build; your
  result may be served from cache.
- `Finished in Ns` under ~0.5s for a `cargo check` after edits: fingerprints, not compilation.
- A peer's ` M` file in the same crate's dependency set is a standing red-source; check
  `git status --porcelain crates/` before trusting any summary.

## Orthogonality

the four recon hits are about a dead PATH shim, reverting dirty files, cron field delivery, and a shell-out terminal restore. None concerns measurement validity. This is the one failure mode where an agent's OWN verification method fabricates the evidence it is asked to trust, which no existing record addresses. (recon R1031065319faa: cargo cached test result)

## Revisions

- 2026-10-31: created
