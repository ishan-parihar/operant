# close the supply-chain gate, the pub-fn tail, and the residual gate failures

Dated-frozen: published `2026-11-20`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

## Why this plan exists

A four-gate survey of `origin/main` at `04e87b4c` (iter-2760) found **two gates red and
two green**, plus a candidate queue that has been shrinking steadily under a peer's
acid-test campaign. This plan closes what is actually left. It does not re-derive work
that already landed, and it states the measured position rather than the intended one.

| gate | state at `04e87b4c` | evidence |
|---|---|---|
| `scripts/clippy-warning-gate.sh` | **RED** until iter-2760, then GREEN | 1 new `dead_code` warning, `gateway/whatsapp.rs` — a write-only `verify_token` field left by iter-2757's setter deletion. Fixed in iter-2760. |
| `scripts/supply-chain-gate.sh` | **RED** | 39 new advisories across 12 crates; allowlist covers 8, all stale |
| `scripts/test-compile-gate.sh` | GREEN | 0 errors; 18 warnings (pre-existing, all in `operant-cli` tests) |
| `scripts/audit-ledger-index.py` | GREEN | index and checkboxes agree (RG-016), id space unique (RG-021) |
| `scripts/genome lint` | GREEN (1 warning) | W014, an owner-gated open question, 17+ days old |
| `scripts/genome doctor` | 1 finding | `docs/handoff/STATE.md` names `413a7a88`, HEAD is `04e87b4c` |
| `scripts/audit-pub-fn-surface.py` | 104 candidates | down from 130 at the start of the campaign; 26 deletions landed |

The supply chain is the only **code** gap. The rest are either already fixed, or are
operator-gated (W014), or are documentation staleness.

## The supply chain: what is actually fixable

`cargo audit` reports **28 vulnerability advisories + 16 informational warnings**. Grouped
by crate and by whether a semver-compatible bump exists **in the lockfile alone**
(`cargo update -p <crate> --dry-run`):

| crate | locked | advisories | bumpable? | note |
|---|---|---|---|---|
| `zstd-safe` | 7.2.4 | 12 | **YES → 7.4.1** | pure lockfile bump, no manifest change |
| `zstd` | 0.13.3 | 1 | **YES → 0.14.3** | same |
| `zstd-sys` | 2.0.16 | 1 | **YES → 2.1.3** | same |
| `pulldown-cmark` | 0.13.4 | 2 | **YES → 0.13.6** | direct dep of `operant-cli` |
| `webpki-roots` | 1.0.8 | 1 | **YES → 1.0.12** | transitive |
| `rustls-webpki` | 0.103.15 | 1 | **YES → 0.103.17** | transitive |
| `quinn-udp` | 0.5.15 | 1 | **YES → 0.5.16** | transitive |
| `tokio` | 1.53.1 | 1 | **YES → 1.54.1** | transitive |
| `rmcp` | 3.5.0 | 3 | **YES → 3.11.0** | transitive via `memory-wire` |
| `rustix` | 0.38.44 | 1 | no (path pinned) | the advisory is about `>=1.0.0`; 0.38.x is unaffected — verify |
| `wasmtime` | 49.0.3 | 13 | **NO** | locked by `extism 1.41.0`, whose latest release pins 49.x |
| `wasmtime-wasi` | 49.0.3 | 2 | **NO** | same |
| `libsqlite3-sys` | 0.30.1 | 1 | **NO** | needs `rusqlite >= 0.40`; 99 call sites (RT-616, already triaged) |

**So 7 crates / 22 advisories are clearable with a lockfile-only bump.** The
remaining 17 are `extism`-locked (15) plus `libsqlite3-sys` (1, already accepted in RT-616)
plus one more `zstd-sys` entry the bump already covers.

That is the whole shape of the work: **one `cargo update`, not a migration.**

## Mechanism

### T1 — the lockfile bump (clears 22 of 39)

**LANDED iter-2762 (`5af57d19`).** Cleared **24** advisories, not 22 -- and the reason is
the most important thing in this plan.

The step as written was impossible: `git check-ignore -v Cargo.lock` reports
`.gitignore:137:*.lock`. A broad `*.lock` rule, added in iter-900 (c2807a1b, "adopt the
project-kb documentation genome") for the kb's flock artifacts, had been swallowing
`Cargo.lock` ever since. Commit c97ac195 then deleted it -- correctly, because a
tracked-but-ignored file is a contradiction.

**That made the gate's central claim false.** Its own header says it audits "the
committed lockfile", and the ledger's RT-10 rows say it twice. No lockfile was committed,
so the gate audited whatever `cargo audit` resolved on the BUILDING machine -- a verdict
no fresh clone reproduces. `git archive HEAD | tar -t` confirms a fresh clone has no
Cargo.lock at all.

Fixed with the narrow negation, not the broad rewrite:

```
*.lock          # kb flock artifacts -- unchanged, still ignores them
!Cargo.lock     # the one file the broad rule must not catch
```

Verified both directions: `Cargo.lock` is now visible to git; an arbitrary `*.lock` is
still ignored (checked with two throwaway names).

**Gate effect: 39 new advisories -> 15.**



1. `cargo update -p zstd-safe -p zstd -p zstd-sys -p pulldown-cmark -p webpki-roots
   -p rustls-webpki -p quinn-udp -p tokio -p rmcp -p rustix`
2. `cargo check --workspace --all-targets` — the whole point of a lockfile bump is that
   it is API-compatible; if it is not, the bump is not semver-compatible and this step
   fails loudly.
3. `bash scripts/clippy-warning-gate.sh` — must stay green.
4. `bash scripts/supply-chain-gate.sh` — must drop from 39 new to ~18.
5. Commit `Cargo.lock` alone. **No manifest change, no source change.**

**Falsifier:** if step 2 fails, the bump is not compatible and the row splits into
per-crate decisions. Do not force it.

### T2 — triage the 18 `extism`-locked advisories

**LANDED iter-2763 (`129acfb2`).** The 15 remaining advisories are all `wasmtime`
(13) + `wasmtime-wasi` (2), via `extism 1.41.0`. **Gate now GREEN: seen=20 allowlist=20
new=0 stale=0.**

Reachability was MEASURED, not assumed. Two facts bound the whole set:

1. `operant-plugins` never names a wasmtime type. It builds an `extism::Plugin`
   (runtime.rs:349) and calls two exports, so exposure is exactly "what a plugin's own
   WASM can make the host do".
2. extism pulls `wasmtime-wasi` with `features = ["p1"]` ONLY
   (extism-1.41.0/Cargo.toml:137). The p2 and wasip3 layers are not compiled.

That resolves five of the fifteen without judgement calls:

| advisory | disposition | evidence |
|---|---|---|
| 0567, 0570 | **not applicable** | "abort in builds WITHOUT the `threads` / `gc` feature" -- `cargo tree -e features -i wasmtime` shows both ENABLED here |
| 0587 | **not applicable** | `os = ["windows"]` in the advisory's own affected data |
| 0420, 0571 | **not applicable** | p2/wasip3 and wasi-http layers, neither compiled (p1 only) |
| 0379 | **not applicable** | aarch64 codegen defect; this host is x86_64 |
| 0378, 0422, 0423, 0424, 0566, 0568, 0569, 0572, 0573 | **accepted** | genuine wasmtime 49 bugs; each needs the plugin WASM to act adversarially, and the operator installs plugins deliberately. Blast radius is the host process, not a privilege boundary. No fix exists here. |

Regression: the gate's own test gains case 5, asserting `Cargo.lock` is not gitignored
AND that `.gitignore` names it explicitly. RED-proven both branches.



All 13 `wasmtime` + 2 `wasmtime-wasi` advisories reach the workspace through
`extism 1.41.0` → `operant-plugins`. `extism` 1.41.0 is its latest release and pins
`wasmtime 49.x`, so there is no semver path.

Options, in cost order:

1. **Record and accept** — each advisory is a `wasmtime` host-side bug (component-model
   type confusion, Cranelift codegen on aarch64, WASI sandbox escapes, unbounded memory
   growth). Reachability must be assessed per advisory against what `operant-plugins`
   actually does: it loads operator-installed WASM plugins. A malicious plugin is the
   threat model, and the operator installs plugins deliberately. Add one allowlist line
   per advisory with that reasoning.
2. **Disable the plugins feature by default** — changes a design preference (AGENTS.md
   says `plugins-wasm` is ON by default). Not this plan's call.
3. **Wait for `extism`** — no release date; record as the fix path.

Recommend (1) + (3), because the gate's purpose is to make a *change* visible, and an
accepted-and-recorded advisory does that. This mirrors RT-616's handling of
`libsqlite3-sys`.

### T3 — `rustix` (folded into T1, kept here for the reasoning)

**LANDED inside iter-2762's bump.** `rustix 1.1.4 -> 1.1.6`. Nothing to allowlist, and
the reasoning below is why.



`RUSTSEC-2026-0486` covers `rustix::fs::{getxattr,listxattr,...}` for `>=1.0.0, <1.1.6`.
The lock has **two** `rustix`: `0.38.44` and `1.1.4`. Only the **1.1.4** one is affected;
0.38.44 is in the advisory's `unaffected` range (`<1.0.0`), which predates the `Buffer`
API entirely.

**MEASURED: the affected one IS bumpable.** `cargo update -p rustix@1.1.4 --dry-run`
reports `Updating rustix v1.1.4 -> v1.1.6`. So this is a **lockfile fix, not a triage**:
the `-p rustix` in T1 covers it and there is nothing to allowlist. A bump is strictly
better than a recorded exception — a stale allowlist entry is reported by the gate and
must be pruned.

It reaches the workspace through two independent paths, both fine with a patch bump:
`cap-primitives` (via `wasmtime-wasi` -> `extism`) and `crossterm` 0.29 (via the TUI).
The second path means this advisory is not purely an `extism` problem.

**Falsifier:** if a later dry-run shows the 1.1.4 path unpinnable, record it as an
accepted advisory with the unreachable-analysis reasoning T2 uses.

### T4 — the pub-fn tail (104 candidates)

**LANDED iter-2767 (`49001811`).** The `tests=0` half is now EMPTY: the auditor reports
exactly one tests=0 candidate, and it is deleted.

**DELETED: `has_user_question_sender` (user_question.rs).** Acid-proven DEAD; 0 references
in `operant-cli`, `operant-os`, `operant-harness`, `operant-api`.

Its doc claimed the clarify tool calls it. It does not. `ClarifyTool` decides by
ATTEMPTING the send -- `try_send_user_question` returns `Some(reply_rx)` when a receiver
exists and `None` otherwise, which is the same information. The predicate was a second
implementation of a decision the code already makes.

The static it read (`USER_QUESTION_TX`) stays live at :79 and :101. This deleted the
accessor, not the cell.

Guard: `the_pub_fn_batch_stay_dead` now 27 needles (was 26); the new one RED-proven by
re-adding the definition shape and restoring byte-identically. 4/4 pass.



The campaign's own plan (`2026-11-20-close-the-pub-fn-surface-queue-...`) names T5 as
"the remaining `tests=0` candidates". Most of T5's named functions are already gone.
The live tail is whatever `scripts/audit-pub-fn-surface.py` reports, and it must be
re-derived rather than trusted from the plan text — that plan's T2/T3/T4 rows were
**already landed by peers** when this session checked them.

Method, per candidate: `python3 scripts/acid-test-pub-fn.py "<file>::<fn>"`, then
**grep `operant-cli` for the name** (the acid test compiles `operant-core --lib` only and
so cannot see a cross-crate caller — MEASURED iter-2757, `with_holon_opt`), then the
clippy gate (a deleted setter can orphan a field — MEASURED iter-2760, `verify_token`).

**Falsifier:** any candidate whose acid test says DEAD but which has a `operant-cli`
reference, or whose deletion orphans a field, is KEPT and recorded in `ACCOUNTED_FOR`.

### T5 — documentation staleness (cheap, closes the last doctor finding)

**LANDED iter-2764 (`08c1a7ec`).** `genome handoff --write` regenerated the handoff zone,
and `genome doctor` now reports **`healthy`**.

The hand-written prose above it claimed "Release line `0.2.1`"; `Cargo.toml` says `0.3.0`
and `check-doc-claims.sh` agrees. A whole minor version stale, in the document whose own
header warns that the prose is the part that goes stale -- the warning was right and the
prose drifted anyway, because nothing derived it. The bullet now names the deriving
command.

Note on that commit: its SUBJECT is wrong (a peer's iter-2764 message) because two agents
wrote commit bodies to the same `/tmp/m2764.txt` seconds apart. Content is correct and
nothing was lost; recorded in iter-2766 rather than amended, per the no-rewrite rule.



1. `./scripts/genome handoff --write` regenerates `docs/handoff/STATE.md` from `HEAD`,
   closing the doctor finding.
2. W014 is owner-gated and 17 days old. **Do not re-ask and do not withdraw** — record
   in the plan that it is blocked on the operator and leave it. It is a warning, not an
   error.

## Gates / success criteria

(fixed before the run)

1. `scripts/supply-chain-gate.sh` exits 0 with `new=0` after T1 + T2.
2. `scripts/clippy-warning-gate.sh` exits 0.
3. `scripts/test-compile-gate.sh` exits 0.
4. `cargo check --workspace --all-targets` exits 0 after the lockfile bump.
5. `./scripts/genome doctor` reports 0 findings.
6. `./scripts/genome lint` reports 0 errors (W014 may remain).
7. Every deletion in T4 has a matching needle in
   `crates/operant-core/tests/dead_pub_fns_stay_dead.rs`, RED-proven.
8. No manifest change in T1. If a manifest change is needed, the plan is wrong.

## Cost

| task | rough cost | why |
|---|---|---|
| T1 lockfile bump | 1 iteration, ~20 min | one `cargo update` + a workspace check; the workspace check is the slow part |
| T2 extism triage | 1 iteration, ~30 min | 15 advisories, one shared root cause, one reasoning paragraph each |
| T3 rustix | minutes | read the advisory, read the lock |
| T4 pub-fn tail | 1-2 iterations | each candidate is one acid test (~2 min) plus a grep plus a gate run |
| T5 docs | 1 iteration | two commands |

**Total: 3-5 iterations.** Nothing here needs an operator ruling except T2 option 2
(disable plugins by default), which is explicitly NOT recommended and NOT taken.

## What this plan does NOT do

- **No operator ruling required** for the recommended path.
- **No design-preference change.** Phonon, memory-wire, sourcehound, the 7 gateway
  platforms, tiered eviction, `plugins-wasm` default-on — untouched.
- **Not the `libsqlite3-sys` major bump.** RT-616 already triaged it with the blast
  radius measured (99 `rusqlite::` sites, 3 crates). It is scheduled on its own merit.
- **Not a blanket allowlist.** Every entry names a specific advisory with a reason,
  per the allowlist's own header. A blanket exemption would defeat the gate.
- **Not re-deriving the peer campaign's landed rows.** Check the tree first; record
  "already landed" rather than redoing the work.