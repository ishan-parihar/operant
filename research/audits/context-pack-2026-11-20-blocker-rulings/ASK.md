# Ruling sheet — the four decisions only the operator can make

Each row is one pending decision. **The sheet lists the decisions, not the answers.**
Where an option is the campaign's own recommendation it is marked `[agent recommends]`;
that is a preference to ratify or reject, not a settled fact.

Operator-owned facts are marked `[operator]` — the repository cannot answer them, and
"not in evidence" is a complete answer for those.

---

## R1 — RT-543: does `operant acp` survive as a second daemon?

**Options on the table**

| option | what it means | status |
|---|---|---|
| (a) deprecate the ACP socket, keep stdio | `operant acp server --socket` prints a loud startup warning naming `operant gateway run --socket` as the replacement; the stdio path stays for editors that speak ACP over a pipe. CHANGELOG + doc edits, NOT removal. | `[agent recommends]` — the prior plan (2026-11-19 post-2668, A1b) recorded this recommendation |
| (b) full retirement | delete `operant acp server` and `operant acp attach` outright | rejected in the prior plan's own text: it breaks the editor stdio integration, and is a larger change |
| (c) keep both, no warning | the status quo | no work, and the duplication stays |

**What is already true (measured at `5af57d19`, not argued)**

- The premise the row was opened on is **FALSE on this tree**. The row's original claim was
  "there are two daemons and neither shares anything". `gateway_runner.rs:6502` calls
  `operant_core::session_server::run_session_socket(session_handler, socket)` on every
  `operant gateway run`, binding `session_client::default_socket_path()`, and
  `operant chat --attach` dials it. The two processes DO share one runtime, one slot table
  and one MCP child set over that socket. **iter-2746 pinned it**:
  `crates/operant-core/tests/rt543_one_socket_shares_one_mcp_child_set.rs` dials ONE
  `run_session_socket` twice, drives a prompt through BOTH connections, and asserts both
  reached the ONE handler.
- So what is open is **no longer the sharing question**. It is the **DISPOSITION**:
  whether `operant acp` (a second daemon with its own agent loop, own registry, own MCP
  children, own AFT bridge, at `crates/operant-cli/src/cmd_acp.rs`) still deserves to exist
  beside a gateway that already serves the client socket, the slots, the child set and the
  channels.
- The ACP protocol itself cannot carry an interactive session (RT-545, closed iter-2658):
  `session/prompt` awaits the whole turn and emits ONE `agent_message_chunk`.
  `session/cancel` acknowledges without interrupting (`acp/mod.rs`, the code names this).
  The two-direction wire protocol shipped on the **session socket**, not ACP.

**What deciding it unblocks:** RT-543 closes. The `acp` command surface
(`docs/library/cli-surface.md`, `docs/library/session-server.md`) stops describing a
second daemon whose relationship to the gateway is undefined.

**What leaving it undecided costs:** every reader who greps `acp` meets a daemon that
duplicates the gateway's job with no statement of which one is canonical. RT-544 and
RT-545 both had to name RT-543 as the owner of this disposition, so two closed rows
point at an open one.

**Owner:** operator `[operator]` — it is a product-surface decision (does a supported
command keep existing), not a code repair.

---

## R2 — RT-280: should a gateway restart resume an interrupted session, and should `/stop` auto-reset?

**Options on the table**

| option | what it means | status |
|---|---|---|
| (a) wire `resume_pending` only | `mark_resume_pending` runs on gateway shutdown; `get_or_create_session`'s existing read branch (which returns the SAME session id) starts working. `suspended` is deleted with its read branches. | `[agent recommends]` — the prior plan's recommendation: "wire `resume_pending` (restart recovery is a real gap) and delete `suspended` unless `/stop`-auto-reset is wanted" |
| (b) wire both | also call `suspend_session` from the live `/stop` arm, so a stopped chat auto-resets on next access | changes what a chat keeps |
| (c) delete both | remove `suspended` + `resume_pending`, their read branches, and `suspend_session` / `mark_resume_pending` / `suspend_recently_active` | closes the row as "not wanted" |

**What is already true (measured at `5af57d19`)**

- Both flags are **persisted, read on every session access, and written by no production
  path**. `PersistentSessionStore::get_or_create_session` is the production path every
  session access takes, and it branches on both.
- The only writers are `suspend_session` (`gateway_session.rs:1174`),
  `mark_resume_pending` and `suspend_recently_active` — and a whole-repo grep finds only
  their declarations and their tests. **No production caller exists.**
- The live `/stop` arm signals the interrupt flag and then writes an **in-memory
  breadcrumb** into the gateway's in-memory `SessionStore` — no I/O at all.
- So both documented behaviours cannot fire: after a gateway restart no session is ever
  resume-pending (the session id is not preserved), and the `/stop` auto-reset never
  happens. `prune_old_entries` skips suspended entries — a branch that can never be taken.

**What is ALREADY FIXED (do not re-open it):** the silent-SQLite-error half. RT-394 closed
it — the four `let _ = self.save_entry(&entry)` swallows are gone, replaced by a source
contract that pins the swallow shape is absent from all four arms. Wiring the feature as-is
would NOT import a silent failure any more.

**What deciding it unblocks:** RT-280 closes, and either the gateway's restart story
becomes real, or a dead persisted column pair stops being carried.

**What leaving it undecided costs:** two persisted columns whose read branches can never
be entered, and an operator reading `suspend_session` believes a `/stop` durable state
exists. The row itself says both are "observable behaviours an operator would notice".

**Owner:** operator `[operator]` — the row names it: *"wiring it is a product decision,
not a patch"*. The sub-question *"how recently active must a session be to be resumed"*
(the sweep takes a `max_age_seconds`) has no answer in the repository.

---

## R3 — RT-190: is cross-surface memory recall wanted at all?

**Options on the table**

| option | what it means | status |
|---|---|---|
| (a) point operant at the serve | one line of config + the `--endpoint` flag (memory-wire v0.8.0) | ruled AGAINST first: it trades the whole in-process advantage (`recall_with_weights` / `retain_doc`, which the HTTP surface does not expose) for a shared store |
| (b) keep in-process, bridge the banks | a one-time bank merge + a sync seam | the deferred tranche |
| (c) accept two families, document the boundary | what shipped at iter-1373 | `[agent recommends]` closing the row as declined: "two families, boundary documented, bridge declined" is a legitimate terminal state |

**What is already true (measured at `5af57d19`)**

- Two disconnected bank families exist **right now on this machine**:
  - operant's own: `$OPERANT_HOME/memory_wire.sqlite` — **5,169,152 bytes**, bank `operant`.
  - the `memory-wire serve` daemon's: `~/.local/share/memory-wire/memory.db` —
    **23,658,496 bytes**, banks `omp` / `memory-wire`.
  - Each surface recalls only its own rows. Knowledge captured in an operant session is
    invisible to every other agent surface (claude-code, codex, opencode, copilot) and
    vice versa.
- memory-wire's own `INTEGRATION_GAPS.md` GAP-4 names this fragmentation for the hermes
  family; the operant half was left "out of this repo" in `RETRIEVAL_EFFICACY_PLAN` §7.
- **RULED option (c) at iter-1373**: the boundary is written into
  `operant.example.toml`'s `[memory]` section. What closed was the operator's confusion
  cost. The row stays OPEN as a future tranche.
- The bank-topology work is **not** blocked by the other memory rows: all four lifecycle
  hooks (RT-191) write through the provider's bank abstraction, so they are
  topology-agnostic — the ruling lands whenever without redoing them.

**What deciding it unblocks:** RT-190 closes either way. If (b) is wanted, it is the only
remaining genuine feature build in the ledger (not a decision, not hygiene) and touches
`memory_wire.rs`, `memory_provider.rs` and the vendor pin.

**What leaving it undecided costs:** the row stays open forever as "a future tranche" with
no owner ruling behind it. The fragmentation itself is real but bounded: nothing is lost,
two stores simply do not cross.

**Owner:** operator `[operator]` — the row says why plainly: *"the serve is shared across
ALL harnesses; moving operant onto it changes what every other agent sees"*. The
repository cannot answer a question about what the operator wants other harnesses to see.

---

## R4 — ASKS Q4: is ~120 lines of input-path terminal probing worth it?

**Options on the table**

| option | what it means | status |
|---|---|---|
| (a) build route (B): own our own fd | open `/dev/tty`, set nonblocking, send `CSI ?u`, read the reply under a bounded deadline BEFORE crossterm's loop starts, plus an explicit surplus-byte policy. ~120 lines + tests on the input path. | `[agent recommends AGAINST]` |
| (b) accept the gap and close the ask | leave the shipped fallbacks as the answer; withdraw Q4 | — |
| (c) keep it open | status quo | open 17 days |

**What is already true (measured, live-verified — not argued)**

- The user-facing harm is **already mitigated**: (i) a trailing-backslash continuation
  needs no keyboard protocol at all — plain Enter on a draft ending in an odd `\` inserts
  a newline (`key_handling.rs`); (ii) Ctrl+J and Alt+Enter take the same branch; (iii) the
  first time a user reaches for the fallback, a once-per-session tip points at
  `/terminal-setup`, which renders per-terminal advice including the tmux
  `extended-keys` fix. All three are unit-tested, and the backslash path was **driven on a
  real pty with NO kitty bytes** and produced a real newline.
- The probe **cannot be copied** from codex (it calls `crossterm::event::buffer_input`,
  which exists only in codex's fork: `codex-rs/Cargo.toml:673`).
- Inferring support at runtime is **impossible**: a terminal that ignored the push and a
  user who really pressed plain Enter send the SAME byte (`\r`), and crossterm parses the
  real flags (`parse.rs:259`) but drops them at the API boundary (`event.rs:251`: only
  `InternalEvent::Event` survives).
- **REJECTED routes, with reasons on record:** second-fd probing (races crossterm, can
  silently eat the operator's first keystrokes); runtime inference (above); forking
  crossterm for one function.

**What deciding it unblocks:** `genome lint`'s only remaining warning (W014, "asks Q4:
open for 17 days") goes away, and the input path either gains a real capability or stops
carrying a 17-day-old open question.

**What leaving it undecided costs:** a gate warning that never clears, and an ask ledger
whose oldest row is a question the campaign has already answered three ways.

**Owner:** operator `[operator]` — it is a cost/benefit judgement about the operator's own
input path: *"is it worth ~120 lines on the input path for the single remaining benefit —
warning a user who never discovers the fallback"*.

---

## What this sheet deliberately does NOT ask

- **The pub-fn queue** (101 candidates, 14 with `tests=0`). Agent work: the acid test
  decides each one, no ruling needed. A plan already owns it
  (`2026-11-20-close-the-pub-fn-surface-queue-one-acid-proven-group-at-a-ti.md`).
- **The red gates that are NOT rulings.** At the pack tip `5af57d19` two gates exit 1: the
  supply-chain gate (15 wasmtime advisories, a triage item — see `# SECTION B-STATE`) and
  `audit-pub-fn-surface.py --check` (101 candidates, the queue above). At the **current
  tip `37c3be43` (iter-2771)** only ONE remains: iter-2763 triaged and allowlisted the
  15 wasmtime advisories (`seen=20 allowlist=20 new=0 stale=0`), so the supply-chain gate
  is green; the pub-fn gate still exits 1 with **103 candidates** (the `tests=0` tail cut
  from 14 to 5 by iter-2767). The other six — doc-claims, ledger-index, supply-chain,
  clippy, test-compile, genome lint — are green.
- **RT-179, RT-285, RT-293, RT-275.** Their code halves landed (RT-285's `discard` is now
  durable — `session/mod.rs:364` deletes from the database and reports the row count);
  the remaining product questions are recorded in ASKS Q5/Q6 and have answers in the tree.
- **The Hydrogen round-2 findings (F1-F12).** Closed and verified; a prior blocker round
  re-derived them and found no live blocker among them.