# EVIDENCE — what has already been measured, and what it bounds

Every figure here is **derived by a command in this round's directory** (kept beside the
spec as `measure_rulings.sh`, so the reviewer or a later round can re-derive it rather than
trust it). Each carries its evidence class.

The three classes that matter below:

- `[LIVE]` — read from the working tree or the running system at `5af57d19`.
- `[CERT]` — asserted by a gate script that exits 0 at this tip.
- `[UNCONFIRMED]` — nobody has measured it; a recommendation resting on it is a
  preference.

---

## E1 — The three open ledger rows are exactly the three rulings [LIVE]

```
rg -c '^- \[ \]' docs/audit/2026-06-19-BUGS.md   → 3
rg -c '^- \[x\]' docs/audit/2026-06-19-BUGS.md   → 659
```

The three: **RT-190** (line 1037), **RT-280** (line 10783), **RT-543** (line 15667).
`python3 scripts/audit-ledger-index.py` exits 0, so the index table and the checkboxes
agree and the id space is unique. [`CERT` — that script]

**What it bounds:** no row is open for want of a *fix*. Every one of the three still
needs code to land behind it -- R2 deletes columns, R3's (b) would build a bridge, R1's
(a) edits `cmd_acp.rs` -- but the *choice* is the blocker, not the engineering. What is
absent is a ruling, not a patch queue.

## E2 — RT-543's premise is false on this tree; its sharing half is pinned [LIVE]

`crates/operant-core/tests/rt543_one_socket_shares_one_mcp_child_set.rs` exists and
dials ONE `run_session_socket` twice, driving a prompt through BOTH connections and
asserting both reached the ONE handler. The row's index entry now says so itself:

> **PREMISE CORRECTED 2026-11-20 ...** the "neither shares anything" half is FALSE on this
> tree.

`gateway_runner.rs:6502` calls `run_session_socket(session_handler, socket)` on every
`operant gateway run`. [`CERT` — the test file exists at this tip and the row's own text
records the correction]

**What it bounds:** the sharing question is **settled**. Only the disposition (does
`operant acp` survive) is open. A reviewer who recommends "make them share" is answering
a question that no longer exists.

## E3 — RT-280's two flags are persisted, read, and never written [LIVE]

```
rg -n 'suspend_session' crates/ --glob '*.rs'
  → gateway_session.rs:386   (a doc comment saying "NOT SET BY ANY PRODUCTION PATH (RT-280)")
  → gateway_session.rs:1174  (the definition)
  → gateway_session.rs:2370, 2839  (tests only)
```

`mark_resume_pending` and `suspend_recently_active` show the same shape: definitions and
tests, no production caller. [`LIVE`]

**What is ALREADY fixed — do not re-open:** the silent-SQLite-error half. RT-394 closed
it; `gateway_session.rs:2027` carries the contract comment ("RT-394: four gateway_session
state changes ... mutated the in-memory entry and then discarded the persist") and the
four swallows are gone. [`CERT` — RT-394's regression module is present]

**What it bounds:** the remaining gap is the *wiring*, not the error handling. A
recommendation that leads with "propagate the SQLite error" is answering a closed row.

## E4 — The two memory banks are both real, both populated, and disjoint [LIVE]

```
$OPERANT_HOME/memory_wire.sqlite          5,169,152 bytes   bank `operant`
~/.local/share/memory-wire/memory.db     23,658,496 bytes   banks `omp` / `memory-wire`
```

Both files exist on this machine right now. Nothing crosses between them by default. The
boundary is written into `operant.example.toml`'s `[memory]` section. [`LIVE` + the
config text]

**What it bounds:** R3 is not hypothetical — the fragmentation is live and both stores are
populated. The serve-side file is **4.58x the bytes** of operant's (23,658,496 /
5,169,152); that is a file-size ratio, NOT a row count, because a SQLite file also carries
indexes, FTS/vector tables and free pages. A reviewer who says "the banks are empty" is
wrong about this machine.

## E5 — The ACP daemon is a real second process, not a stale command [LIVE]

`cmd_acp.rs` builds its **own** runtime: `AcpRuntime::start(config.clone())` — "The
runtime is built ONCE for the life of the server: one database, one MCP child set, one
registry, one agent." The `attach` subcommand builds none. [`LIVE`]

The ACP protocol cannot carry an interactive session (RT-545, closed iter-2658):
`session/prompt` awaits the whole turn and emits ONE `agent_message_chunk`; `session/cancel`
acknowledges without interrupting, and the code names this. The two-direction wire protocol
shipped on the **session socket**, not ACP. [`CERT` — RT-545's closing record]

**What it bounds:** a recommendation to "add interactivity to ACP" contradicts a closed row
that measured it as architecturally impossible on that protocol.

## E6 — The enhanced-keys fallbacks are shipped and live-verified [CERT]

ASKS Q4 records: the trailing-backslash continuation needs no keyboard protocol at all;
Ctrl+J and Alt+Enter take the same branch; a once-per-session tip points at
`/terminal-setup`. "All three are unit-tested, and the backslash path was **driven on a
real pty with NO kitty bytes** and produced a real newline." [`CERT` — the ask row's own
record, written from a live pty run]

The probe cannot be copied from codex (`crossterm::event::buffer_input` exists only in
codex's fork). Runtime inference is impossible: a terminal that ignored the push and a
user who really pressed plain Enter send the SAME byte (`\r`). [`CERT` — `crossterm`
`event.rs:251` drops everything but `InternalEvent::Event`]

**What it bounds:** R4 is a *cost/benefit* question, not a feasibility one. The benefit is
narrow (warn a user who never discovers the fallback); the fallbacks are already there.

## E7 — The pub-fn queue is agent work, not a ruling [LIVE]

```
candidates (0 production callers) : 104
  accounted for                   : 3
  NEW                             : 101     (14 with tests=0)
```

A plan already owns it and its method is settled: **the acid test** — comment out `pub`,
`cargo check -p operant-core --lib`, restore. A clean compile with only a `dead_code`
warning is the only proof a function is dead. [`LIVE`; the plan is
`docs/plans/2026-11-20-close-the-pub-fn-surface-queue-one-acid-proven-group-at-a-ti.md`]

**What it bounds:** a reviewer who recommends "delete the dead pub fns" without the acid
test has proposed something the project already does properly. And the auditor itself says
a candidate is a *census result, not a verdict*.

## E8 — What has NOT been measured [UNCONFIRMED]

These are named so a recommendation cannot silently rest on them:

- **The ACP daemon's real-world usage.** Nobody knows whether any editor or user actually
  speaks ACP to `operant acp`. There is no telemetry and no caller in the tree. The
  disposition ruling therefore rests on a **preference** about supporting a surface
  nobody is measured to use.
- **The operator's actual pain from the memory-bank split.** Nobody has measured whether
  a fact captured in operant was ever *needed* from another harness and vice versa. R3's
  (b) rests on an unmeasured need.
- **The cost of a gateway restart losing a session.** Nobody has measured how often a
  restart happens mid-session, or how much work is lost. R2's (a) rests on an unmeasured
  frequency.
- **Whether Shift+Enter is actually missed.** The fallback tip fires, but nobody has
  measured how often a user hits the fallback and is annoyed. R4's (a) rests on an
  unmeasured annoyance.

A recommendation that treats any of these as `[LIVE]` is a defect in the reply.