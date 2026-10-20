# Audit: the 2026-10-19/20 defect sweep — what was found, what was fixed, and what remains

Date: 2026-10-20. Agent: `omp`.
Range: iters 980–1022 (66 commits) on `origin/main`.
Ledger: `docs/audit/2026-06-19-BUGS.md` (the open rows are S1 and S2 only).

This is the morning review. It names every defect the sweep found, how each
was proven, and what is deliberately left open.

## How to read the evidence

Every claim below is backed by a **mutation check**: the fix is reverted and
the named test is shown to FAIL. A test that passes is not evidence; a test
that fails when the fix is removed is.

Three defects in this sweep were caught by the sweep's OWN tests being
wrong — they passed for the wrong reason, and the second pass made them
bite. Those are called out, because the shape recurs.

## The defects, by class

### Security

| Row | Defect | Evidence |
|---|---|---|
| RT-42 | The dashboard served its session token on an UNAUTHENTICATED route. `GET /` returned it, and the scraped token opened every "protected" route. | Reproduced against the running server; the fix is a token-gated index + an HttpOnly cookie + a Host-header check. Two mutations caught. |
| RT-43 | The dashboard's API read only `Authorization: Bearer`. The bundled frontend sends `X-Hermes-Session-Token` and never sends `Authorization` — so every API call the real UI made returned 401. The dashboard was non-functional. | Read out of the bundle itself; the test asserts the bundle's header name. |
| RT-70 | The shared updater's checksum guard matched only `SHA256SUMS`; `cortexkit/aft` publishes `checksums.sha256`. The verification **never ran for aft**. The first-install path (which never went through the updater) downloaded and executed an unverified binary, with a non-atomic write. | Verified against the live release's asset list. |
| RT-72 | `rustls::ClientConfig::builder()` PANICS when the crate features are ambiguous — and `hyper-rustls` (via reqwest) enables `aws-lc-rs` while `tokio-rustls` enables `ring`. The Email adapter panicked on every send; the proxy's TLS setup broke every proxied WebSocket. | Reproduced at `rustls-0.23.46/src/crypto/mod.rs:249`. The fix names the provider at the call site, so it works under every feature combination. |
| RT-73 | The SMTP client read every server reply and DISCARDED it: a `535`, a `550` and a `552` all produced `Ok(())` — a successful send for a rejected message. | Every step now checks the status code. |
| RT-54 | A webhook adapter with no signing secret accepted every POST. The configuration is legitimate (a private-network receiver), but nothing said so. | A startup warning naming the address and the two fixes. |

### Correctness

| Row | Defect | Evidence |
|---|---|---|
| RT-69 | The context eviction could not meet the budget and said nothing; the reactive overflow path re-issued the request and earned the same provider 400. | Four mutations, each run separately. The pre-existing `loop_recovery_paths` test caught the over-strict first fix — the rule is a two-strike rule because of it. |
| RT-67 | An MCP tool's registry name was not injective: server `a` + tool `b_c` and server `a_b` + tool `c` produced the same name. The registry REPLACES on a name collision and the sync tears down BY NAME, so a hostile MCP server could shadow another's tool. | Fixed; then iter-1018 found that iter-1014's own "injective by construction" claim was FALSE (`-` and the literal `_2d_` collided) and corrected it. |
| RT-68 | The gateway session store's `prune_old_entries` existed since it was written and its only callers were its own tests — so every conversation the gateway ever saw stayed forever. | A periodic sweep plus `[gateway] session_retention_days`. The mutation: emptying the sweep body fails the test. |
| RT-66 | The CLI and the core resolved the operant home differently on Windows, so the startup `.env` loader and the per-turn reload read two different files. | One resolver. |
| RT-55/56 | The supply-chain gate could not report the finding it exists to report (it treated cargo-audit's non-zero exit as "the DB is unreadable"), and a size-bound checkpoint skip was reported as the wrong cause. | The gate's stub now exits non-zero, which is what made the path reachable at all. |
| RT-52 | The TUI corpus had been RED for nine days and no gate ran it. | `scripts/self-test.sh` runs it now; the corpus is green (64 gated, 0 drift). |
| RT-58 | The checkpoint store snapshotted itself. | Closed. |

### The claim audit (RT-45 … RT-53)

A separate class: **documentation that asserts something the code does not
support.** Every AGENTS.md, README and CHANGELOG claim that named a crate, a
command, a count or a tool that no longer existed was corrected against the
code. The one that mattered most: AGENTS.md described `operant-channels` and
`operant-gateway` as live crates (they were deleted in a purge), and the
crate tree named 12 crates as 2.

**A claim is worth exactly what the search behind it covered.** A README
claim that operant had no MCP server was "corrected" from a grep over
`operant-core/src/mcp.rs` — the server is in `crates/operant-cli/src/mcp_serve.rs`
(955 lines), and the evidence was in hand at the time (`operant mcp --help`
listed `serve`). That correction was reverted, and the lesson recorded.

## The three traps this sweep paid for

1. **A test that passes is not evidence.** The gateway-sweep test, the
   context-overflow test and the MCP injectivity test each passed for the
   wrong reason at first — the first proved the BUILD path rather than the
   reactive one, the second proved a re-implementation rather than the
   production path, the third was vacuous. Each was rewritten until a
   mutation failed it.

2. **A "prune" that nothing calls is not a prune.** RT-68's function was
   correct and tested; the defect was that its only callers were its own
   tests. The class: look for the CALLER, not the implementation.

3. **A verification that silently never runs reads as protection.** The AFT
   checksum guard matched a filename upstream does not use, so it never
   fired — and the module doc said it verified.

## What remains open

| Row | Why it is not agent-actionable |
|---|---|
| S1 (OSC-8, pinned-image post-draw writes) | Needs a live terminal to observe. |
| S2 | The same. |
| CR-6 (the stale `/usr/local/bin/operant`) | Needs `sudo`, which this session does not have. |
| RT-26/27/28 (dependency advisories) | Owner decisions, recorded in AD-033/AD-039. |
| RT-44 (the 75 missing dashboard routes) | A project, and the owner's call. The bundle is from a larger harness; the server implements 4. |
| The 1.5 GB checkpoint store | Reclaimable with `operant checkpoints clear` (RT-61); the forward-looking bound is in place. |
| Session/DB retention policy | A policy decision, not a defect. |

## The gates, as they stand

| Gate | State |
|---|---|
| `cargo test --workspace --all-features` | green |
| `scripts/clippy-warning-gate.sh` | `seen=24 allowlist=24 new=0 stale=0` — pass |
| TUI corpus (`_run.py verify`) | 64 gated, 0 drift; 7 surfaces parked in `excluded.json` with a reason each |
| `scripts/genome lint` | clean |
| `docs_lint` | PASS — 0 hard failures |
| `scripts/genome doctor` | healthy |

## What a new agent should know before touching this tree

- The clippy gate and the TUI corpus are BOTH wired into
  `scripts/self-test.sh`. A red corpus now means a real regression.
- `prove` and `verify` must never run concurrently — `prove` rewrites the
  goldens a concurrent `verify` is reading.
- `verify` must never run against a binary a build is writing. The tell is
  the GATED COUNT: 64 is healthy, lower means the binary was not answering.
- A golden must never encode a transport-library error string.
- The `~/.local/bin/operant` deploy is verified by three matching md5s: the
  deployed file, the build artifact, and `/proc/<MainPID>/exe`.
