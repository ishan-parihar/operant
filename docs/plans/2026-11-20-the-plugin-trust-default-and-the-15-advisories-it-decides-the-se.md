# The plugin trust default, and the 15 advisories it decides the severity of

Dated-frozen: published `2026-11-20`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

Written by the operator agent at `origin/main` = `5af57d19` (iter-2762). Every number
below was MEASURED in a CLEAN worktree at that tip before it was written down.

## Claim rule

Never take a number from a header or a previous plan. `git fetch origin`, read the top
label from `origin/main`, add one. Re-run every command in the baseline before trusting
the row that cites it.

## Why this exists next to iter-2761's plan

`docs/plans/2026-11-20-close-the-supply-chain-gate-the-pub-fn-tail-and-the-residual.md`
(iter-2761) already owns the supply-chain gate, and its T1 has LANDED. MEASURED at this
tip: `zstd-safe` 7.4.1, `pulldown-cmark` 0.13.6, `webpki-roots` 1.0.12, `tokio` 1.54.1 —
the lockfile bump is in, and the gate went from 39 new advisories to **15**.

Its T2 covers the remaining 15 as a TRIAGE: read each, record it, allowlist it. That is
correct and this plan does not repeat it.

What that plan does NOT decide is the question its own T2 option 2 names and sets aside:
"**Disable the plugins feature by default** — changes a design preference. Not this plan's
call." This plan is that call, made on evidence, plus the one finding that changes the
severity of all 15.

## The finding: signature verification is OFF by default, so the plugin surface is untrusted

`crates/operant-plugins/src/signature.rs:473-475` pins it as a test:

```rust
#[test]
fn test_signature_mode_default_is_disabled() {
    assert_eq!(SignatureMode::default(), SignatureMode::Disabled);
}
```

and `host.rs:40` uses it directly:

```rust
Self::with_security(workspace_dir, SignatureMode::Disabled, Vec::new())
```

So the default host loads WASM plugins with **no signature check**, and `plugins-wasm` is
ON by default (`operant-cli/Cargo.toml:94`, `default = ["plugins-wasm"]`, with the
comment "a default-off gate meant the whole plugin architecture ran in no build the
operator actually launches"). MEASURED dependency chain:

```text
$ cargo tree -p extism -i wasmtime
wasmtime v49.0.3
├── extism v1.41.0
│   └── operant-plugins v0.3.0
│       └── operant-cli v0.3.0
```

`extism` 1.41.0 IS its latest release (MEASURED: `cargo search extism` -> 1.41.0), and it
pins `wasmtime 49.x`, so there is **no semver path** — the peer plan is right about that.

That combination is what makes this a decision rather than a tidy-up. A wasmtime host-side
bug (sandbox escape, type confusion, memory growth) is only reachable if a plugin runs,
and a plugin runs if the operator installs one. The threat model is "the operator
deliberately installed a plugin", and the signature mode says operant does not check who
wrote it.

## The rows

### T1 -- decide the signature default, and write the reason down

Three positions are defensible; the plan's job is to pick one and record why.

(a) **Keep `Disabled` and record it.** The operator chose to install the plugin; a
    signature check on a local directory install verifies nothing the operator did not
    already vouch for. Cost: zero code. Requires: a doc line saying the default is
    deliberate, because today a reader finds `Disabled` as a bare `Default` impl with a
    test that only pins the behaviour, not the reasoning.

(b) **Default to `Permissive`.** MEASURED: the real variant is `Permissive`, not `Warn`
    (signature.rs:16-23), and it HAS a real code path -- enforce_signature_policy's
    `(None, _) | (_, None)` arm logs `tracing::warn!("plugin is unsigned; loading in
    permissive mode")` and returns `Ok(VerificationResult::Unsigned)` (signature.rs:208-213).
    A plugin with no signature or an unknown key loads and says so. That is strictly more
    information for the operator and does not change what runs. Cost: one `#[default]`
    attribute moved, plus the tests that assert `Disabled` (signature.rs:475, host.rs:40
    and the fixtures at 636/656) updated in the same commit.

(c) **Default to `Enforce`.** Refuse unsigned plugins. This is a behaviour change that
    would break every existing operator install. Almost certainly wrong as a default and
    this plan says so, but the reasoning belongs in the record rather than in someone's
    head.

THE VARIANTS, MEASURED (signature.rs:16-23): `Strict` (reject unsigned), `Permissive`
(warn but allow), `Disabled` (no check) -- with `#[default]` on `Disabled`.
`enforce_signature_policy` returns early `Ok(Unsigned)` for `Disabled` (signature.rs:199)
and only branches on `Strict`/`Permissive` after that. MEASURED: `host.rs:103`'s
`_ => SignatureMode::Disabled` is NOT a catch-all on the enum -- it is the string parser
`parse_signature_mode` (unknown string -> `Disabled`), so it does not override `Default`
and changing `#[default]` takes effect for any caller that does not pass an explicit mode.
But `host.rs:40` passes `Disabled` EXPLICITLY, so it must be updated too or it will pin the
old behaviour regardless of the `Default`.

RECOMMENDATION: (b) -- the variant exists, its code path is live, and it is a strict
information gain with no behaviour change. Not (c), which would break existing installs.

### T2 -- triage the 15 with the signature finding in the reason

Each allowlist line must say what the advisory is AND what makes it acceptable HERE.
Generic reasons ("transitive, widely held") are what the allowlist header explicitly
warns against ("This is NOT a blanket exemption"). For each of the 15, the reason has
these components and the differences matter:

- **Reachability.** Is it a host-side bug that fires only while running a plugin, or does
  it fire at compile time / on load? `wasmtime-wasi` advisories (RUSTSEC-2026-0587) are
  about the WASI sandbox; `wasmtime` ones split between codegen and the component model.
- **What runs.** MEASURED: `operant-plugins` loads operator-installed WASM from a
  configured directory. There is no auto-download of plugins in this tree (verify before
  asserting it — if one exists, the reachability argument changes completely).
- **What the fix would be.** `extism` has no newer release; the fix path is upstream.

Record as a new ledger row (the pattern is RT-616's: one row, each advisory named, each
reason stated, the gate going green with the deltas still fail-able).

### T3 -- make the triage FALSIFIABLE

An accepted advisory is a promise that the reasoning still holds. The allowlist has no
expiry and the gate only reports STALE when the advisory disappears upstream. So add one
cheap thing: a line in the triage row naming what would change the verdict (a new `extism`
release, a plugin auto-download path appearing, `plugins-wasm` going default-off). That is
prose, not machinery, and it is what stops the entry from becoming the next 2-year-old
unexamined allowlist line.

## Gates / success criteria

(fixed before the run)

1. `bash scripts/supply-chain-gate.sh` exits 0 with `new=0`, and `stale=0`.
2. Every one of the 15 has its own line in `.ci/advisory-allowlist.txt` with a reason
   naming reachability, not a category.
3. `docs/audit/2026-06-19-BUGS.md` gains a row with all 15 named, and
   `python3 scripts/audit-ledger-index.py` still prints OK.
4. If T1 changes the default: `cargo test -p operant-plugins` passes, and the tests that
   pinned `Disabled` are updated in the SAME commit (not left red for the next agent).
5. `cargo check --workspace --all-targets` exits 0.
6. All six gates stay green: audit-config-surface, audit-config-keys, supply-chain,
   audit-ledger-index, check-doc-claims, genome lint.
7. If the answer to T1 is "keep Disabled", that decision is written in the doc comment on
   `SignatureMode::default`, not only in this plan — a plan nobody reads is not a record.

## Cost

T2 is the bulk: 15 advisories, one shared root cause, one paragraph each. T1 is a decision
plus a doc line, or a small enum change if `Warn` exists. T3 is one paragraph.

Estimate 2 iterations.

## Order and dependency

```text
T1 (decide the default)  -->  T2 (triage, with the T1 answer in the reason)
T3 (falsifiability note) -->  independent, cheap
```

T1 first, because its answer is an input to every T2 reason.

## What this plan explicitly does NOT do

- It does not re-do iter-2761's T1 (the lockfile bump) — MEASURED landed.
- It does not re-list the 15 as a bare "accept all"; each gets a reachability reason, and
  any that turns out to be unreachable-until-a-plugin-runs says so explicitly.
- It does not change a DESIGN PREFERENCE silently. If the answer is (a), that is recorded
  as a decision with its reasoning, not as an omission. AGENTS.md's "plugins-wasm is ON by
  default" stands either way; what this plan touches is the SIGNATURE default, which is a
  different knob.
- It does not touch the 5 kernel-sidecar failures (environmental, need an untracked
  `.venv`), the 3 owner-gated ledger rows (RT-190/280/543), or iter-2710's pub-fn queue.
- It does not claim the suite is red — MEASURED: 3689 passed / 5 failed, all five
  environmental, at this tip. The supply gate is the only red thing here.

## If the operator wants nothing further

The supply gate is the last red gate I can find at the tip, and this plan closes it. After
that the honest remainder is unchanged: 3 owner-gated rows, 5 environmental kernel tests,
and the owned pub-fn queue. The session can close with no known-red gate.