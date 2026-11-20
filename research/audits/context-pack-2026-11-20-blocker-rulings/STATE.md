# STATE — what operant says about itself right now

**Subject line: operant** — a Rust agentic harness at
`github.com/ishan-parihar/operant`, release line `0.3.0`, 7 workspace crates,
**2,116 `pub fn`s** in `operant-core` production code
(`python3 scripts/audit-pub-fn-surface.py --check` prints `pub fns in operant-core
(production code) : 2116`, and `check-doc-claims.sh` asserts the 7) [`CERT`].

If your reply is about anything not named here, say so and stop.

This section is the subject's own self-description, quoted from its live artefacts. It is
here to be **audited for staleness** — every figure below carries the artefact it came
from and the command that derives it.

---

## What operant is

A single Rust binary (`operant`) that runs an agent loop over a model, with:

- **7 workspace crates** (`Cargo.toml`): `operant-core`, `operant-cli`, `operant-config`,
  `operant-api`, `operant-os`, `operant-markdown`, `operant-macros`.
  [`CERT` — derived from `Cargo.toml` `members`; `check-doc-claims.sh` asserts this and
  prints `workspace members: 7`]
- **53 CLI command variants** with real handlers (`main.rs` `Commands` enum).
  [`CERT` — `check-doc-claims.sh`: "CLI command variants: 53; CLI list: 53 entries,
  exact match"]
- **7 gateway platform adapters**: telegram, discord, slack, whatsapp, email_smtp,
  sms_twilio, webhooks. [`CERT` — `check-doc-claims.sh`: "gateway platforms: 7"]
- **memory-wire** as the only memory engine, in-process, sqlite-backed.
- **sourcehound** as the browser/web tool engine, driven over its MCP server (stdio).
- **Phonon** (GGUF q8_0) as the default TTS provider.
- A **TUI** (`ratatui` + `crossterm`) with a prompt composer, ghost text, `@file:`
  references, vim motions, and a word-geometry nav model.
- An **autonomous coding mode** (`operant autonomous`).

## What the iteration model is

Every change is one iteration: `feat(iter-N)` / `fix(iter-N)`, committed, pushed to
`origin/main`, then a release build and a deploy to `~/.local/bin/operant`. The operator
pulls from `origin/main` and tests locally. **A change that lives only in a working tree
is not done.** [`CERT` — `AGENTS.md` "The Iron Rule"]

Multiple agents work the same tree concurrently. The rules that exist because of that:
the iteration label is derived from `origin/main` (never local history), the shared
working tree may carry a peer's uncommitted WIP, and **fleet rule 7** says a gate that is
red from a peer's slice is proved-delta-zero rather than fixed. [`CERT` — `AGENTS.md`
"Parallel-Agent Coordination Protocol"]

## The gate battery, measured at `5af57d19` (iter-2762) and re-measured at `37c3be43` (iter-2771)

Run in a **clean throwaway worktree** at the tip, `source scripts/dev-env.sh` applied:

| gate | exit | note |
|---|---|---|
| `bash scripts/check-doc-claims.sh` | 0 | every derived AGENTS.md claim matches the code |
| `python3 scripts/audit-ledger-index.py` | 0 | index and checkboxes agree; id space unique |
| `bash scripts/supply-chain-gate.sh` | **0** at `37c3be43` (**1** at `5af57d19`) | At `5af57d19`: 15 new `wasmtime` / `wasmtime-wasi` 49.0.3 advisories (`seen=20 allowlist=5 new=15 stale=0`). **`iter-2763` closed it**: allowlist now `seen=20 allowlist=20 new=0 stale=0`. The advisories are the known un-bumpable set (extism 1.41.0 pins wasmtime 49.0.3). |
| `bash scripts/clippy-warning-gate.sh` | 0 | no new warnings vs the allowlist |
| `bash scripts/test-compile-gate.sh` | 0 | every kept crate's TEST targets compile |
| `./scripts/genome lint` | 0 | **1 warning** (W014, ASKS Q4, 17 days open) |
| `python3 scripts/audit-pub-fn-surface.py --check` | **1** | `37c3be43`: **103 candidates** (was 101 at `5af57d19`; the `tests=0` tail was deleted down to **5** by `iter-2767`). Still exit 1. |
| `./scripts/genome doctor` | 0 | 1 finding: `STATE.md` names `413a7a88`, HEAD is `5af57d19` (333 non-documentation paths changed since) |

**Two red gates at `5af57d19` became ONE red gate at `37c3be43`.** The supply-chain
queue is closed (iter-2763 triaged and allowlisted the 15 wasmtime advisories); the
pub-fn queue is still open, and the `tests=0` subset is down from 14 to 5 after
iter-2767 deleted 27 of 27 candidates.

`cargo test -p operant-core --lib` at `6a90b5c4` (an earlier tip; not re-run at
`5af57d19`): **3689 passed / 5 failed**, and all 5
are `tools::kernel::tests::*` failing only because a fresh worktree has no
`kernel-sidecar/.venv` and no `vendor/prime-agent` submodule. In the main tree those 5
pass. [`PRIOR` — measured in the main tree on an earlier tip, not re-run at this one;
AGENTS.md fleet rule 6 is the policy that explains the worktree false red, not a gate run]

## The ledger: 3 open rows, all owner-gated

`rg -c '^- \[ \]' docs/audit/2026-06-19-BUGS.md` → **3**; `^- \[x\]` → **659**.
The three open rows are **RT-190, RT-280, RT-543** — every one of them a product
decision rather than a code repair. [`CERT` — derived at `5af57d19`]

## The three surfaces this round asks about, in the project's own words

- **`operant acp`** — "Run the ACP (Agent Control Protocol) server", with
  `--socket <PATH>` to "Serve over this UNIX socket instead of stdio", and an
  `attach` subcommand whose "client is thin: it opens the socket, sends the command and
  prints the answer. It builds NO runtime of its own." [`CERT` — `cmd_acp.rs` doc comments]
- **`PersistentSessionStore`** — two persisted flags with live read branches in
  `get_or_create_session`: `suspended` ("When true, next access auto-resets (from
  /stop)") and `resume_pending` ("Session was interrupted by restart, resume on next
  access"). The code now says plainly they are **NOT SET BY ANY PRODUCTION PATH**.
  [`CERT` — `gateway_session.rs:386` and the field docs]
- **memory** — `operant.example.toml`'s `[memory]` section states the boundary:
  operant's store is `$OPERANT_HOME/memory_wire.sqlite` (bank `operant`), a
  `memory-wire serve` daemon's is `~/.local/share/memory-wire/memory.db` (banks `omp` /
  `memory-wire`), and nothing crosses by default.
  [`CERT` — `operant.example.toml`]

## What the project believes about its own maturity

`docs/handoff/STATE.md` is the "where does the project stand right now" document. It
currently names `413a7a88` while HEAD is `5af57d19`, with 333 non-documentation paths
changed since — `genome doctor` reports this as its one finding. [`LIVE`]

`docs/library/` is the mechanism library: one page per architectural element, changed in
the same iteration as the code it describes. `docs/decisions/` (44 AD records) and
`docs/regressions/` (21 RG records, plus 1 retired) are immutable once numbered — a correction is a new
record via `genome rec supersede`, never an edit. [`CERT` — `genome doctor`: "records: 44
AD + 21 RG active, 1 retired"]