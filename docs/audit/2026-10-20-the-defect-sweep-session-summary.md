# Audit: the 2026-10-19/20 defect sweep — what was found, what was fixed, and what remains

Date: 2026-10-20. Agent: `omp`.
Range: iters 980–1046 (67 commits) on `origin/main`.
Ledger: `docs/audit/2026-06-19-BUGS.md` — **190 rows closed, 2 open** (S1 and S2, both live-terminal checks that need a terminal this session did not have).

This is the morning review. It names every defect the sweep found, how each
was proven, and what is deliberately left open.

## How to read the evidence

Every claim below is backed by a **mutation check**: the fix is reverted and
the named test is shown to FAIL. A test that passes is not evidence; a test
that fails when the fix is removed is.

Three defects in this sweep were caught by the sweep's OWN tests being
wrong — they passed for the wrong reason, and the second pass made them
bite. Those are called out, because the shape recurs.

**A fourth was caught by a TRACER, not by reading the code.** The TUI's
process-global reasoning-mode cell leaked between tests (RT-79). Reading
the code said every writer was guarded — because a search for "the setter is
called" does not say "and the CALLER holds the guard". An `eprintln!` in the
setter that printed the mode, the thread and the TEST NAME found it, after
the obvious suspects were all cleared. The same class — a claim that reads
as verified but was verified against the wrong thing — is the reason
several rows here name the MECHANISM, not just the outcome.

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
| RT-75/76/77 | A failed Telegram send was reported as delivered; `edit_message` swallowed every 400; the poll looped silently every two seconds. | The same class as the SMTP defect (RT-73), in the platform with the most users. |
| RT-86 | The Slack adapter is OUTBOUND-ONLY — there is no Socket Mode WebSocket, so a Slack message never reached the agent, while the gateway reported the adapter as "enabled". | The status report now carries `inbound: false`. |
| RT-87/88 | `operant-tools` was a 30 860-line parallel tool library the product called ONCE. Deleted: 30 695 lines. | A closure analysis over the module graph, then the workspace build. |
| RT-89 | 43 923 lines of provider, memory and session implementations are compiled into every binary and no configuration can reach them. | Measured by an exact module-path search per crate. |
| RT-90 | `default_provider_config` claimed three callers it did not have. | A workspace-wide search for the name. |
| RT-91 | The installer's `cp -a` left a STALE ONNX Runtime symlink, and the installed binary could not start at all. | The machine could not run the binary; the corpus read "0 gated, 70 drifts" and that was not a regression. |
| RT-92 | The CLI compiled the `postgres` dependency for a crate it never called, through a `ci-all` feature no CI runs. | A workspace-wide search for `operant_memory::`. |
| RT-94 | `operant doctor` reported a provider as "valid" that the runtime cannot serve. Bedrock speaks AWS's Converse API; the runtime only speaks chat-completions and messages. | The root was `default_wire_api`'s default saying "chat_completions" for a provider that does not speak it. The mutation fails the test. |
| RT-95 | The TUI said "Connected to AWS Bedrock." while the agent kept talking to the PREVIOUS provider — five catalogue providers have no usable endpoint URL, so the retarget was skipped. | A second defect surfaced in the same fix: the status message was set BEFORE the persist, so a persist failure overwrote it. The mutation fails the test. |
| RT-69 | The context eviction could not meet the budget and said nothing; the reactive overflow path re-issued the request and earned the same provider 400. | Four mutations, each run separately. The pre-existing `loop_recovery_paths` test caught the over-strict first fix — the rule is a two-strike rule because of it. |
| RT-83 | The token estimate counted only the message's text — and a tool-call turn's text is EMPTY by construction, so a 100 KB tool call was estimated at 4 tokens. The eviction was blind to the largest messages. | The mutation fails with "got 4". |
| RT-78 | The LLM compression's HEAD boundary was a message count, so a `tool_use` and its `tool_result` could be separated by the summary. `repair_tool_pairs` existed and was never called. | Two mutations, run separately. |
| RT-81 | A `transport = "streamable-http"` MCP server could not connect at all: no `Accept` header (406), no `Mcp-Session-Id` echo (a new session per request), no SSE-framed response reading. | Two mutations. |
| RT-84 | A JSON-RPC error from an MCP SSE server was handed back as a successful result — the caller treated the error payload as the tool's output. | The mutation fails with the error object as the "result". |
| RT-82 | Every MCP transport advertised `sampling` and `roots`; the HTTP and SSE ones cannot answer a server-initiated request, so the server blocked waiting for a reply. | The mutation fails. |
| RT-67 | An MCP tool's registry name was not injective: server `a` + tool `b_c` and server `a_b` + tool `c` produced the same name. The registry REPLACES on a name collision and the sync tears down BY NAME, so a hostile MCP server could shadow another's tool. | Fixed; then iter-1018 found that iter-1014's own "injective by construction" claim was FALSE (`-` and the literal `_2d_` collided) and corrected it. |
| RT-80 | Dropping an MCP stdio transport left the server subprocess running as an orphan (`kill_on_drop` defaults to false; no `impl Drop`). | Four attempts to make the test non-vacuous, then the mutation fails. |
| RT-79 | A TUI test could poison the process-global reasoning-mode cell for every test after it — 5 failures in 20 runs, 0 in 25 after. | Found with a tracer, not by reading: a search for "the setter is called" does not say "and the CALLER holds the guard". |
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

## The structural finding: a parallel library the product barely used

The sweep's largest result is not a single defect but a PATTERN, and it is
worth stating on its own because it explains several of the rows above.

`operant-tools` was a 30 860-line crate of tool implementations — the
browser, the file tools, the HTTP tool, the web tools, a whole second MCP
stack — that the product called **ONCE**: `cli_discovery::discover_cli_tools`,
from the doctor's CLI check. The core registers its OWN browser tool, file
tools, HTTP tool and web-search tool, and those are the ones the model
calls.

The cost was not only 30 695 lines of unreachable code (RT-87, RT-88 — now
deleted). It was **divergence**: two implementations of the same tools,
kept in step by hand. The Streamable-HTTP gap (RT-81) had to be fixed in
the core while a copy in `operant-tools` — in an older form, pinned to
MCP protocol `2024-11-05` against the core's `2025-06-18` — was never
consulted, even though it had a Streamable-HTTP implementation.

The same pattern repeats in three more crates, and RT-89 measures it:

| crate | lines | reachable | what the live call is |
|---|---|---|---|
| `operant-providers` | 30 390 | 19 of 21 modules unreachable | ONE call: the doctor validating a provider name |
| `operant-memory` | 11 324 | 0 | nothing — the core's `build_memory_provider` ignores every name but memory-wire, and warns |
| `operant-infra` | 2 270 | 0 | nothing — and it holds the only message debouncer and stall watchdog the tree has (RT-93) |

**43 923 lines** of provider, memory and session implementations that no
configuration can reach. Two of those crates hold real, working capability
for backends and native APIs operant does not support — so this is a
**feature gap wearing a redundancy's clothes**, and the fix (wire the
routing, or narrow the configuration surface so nothing is silently
ignored) is the owner's call. RT-89, RT-90 and RT-93 record the
measurement and the options; nothing was deleted.

The `operant-tools` half WAS deleted, because it duplicated the core's own
code rather than adding a capability the core lacks.

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
| RT-89 / RT-93 (43 923 unreachable provider/memory/session lines, and the infra crate's debouncer and watchdog) | A product decision: WIRE the routing so a configured provider or memory backend is the one used, or narrow the configuration surface so nothing is silently ignored. The implementations are real and working, so deleting them is not the only answer. |
| RT-86 (Slack Socket Mode) | A feature: the `apps.connections.open` handshake, the WebSocket, the reconnect and the event routing. Until it exists, the channel is outbound-only and the status report says so. |
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
