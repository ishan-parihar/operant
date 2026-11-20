# VERIFICATION — the rulings round, checked against the artefacts before adoption

**Round:** `context-pack-2026-11-20-blocker-rulings` (round 4, a rulings round)
**Venue:** Google AI Studio, `gemini-4-hydrogen` at `xhigh`
**Pack:** 41,351 B (~13.8k tokens); sent inline as one message (46,763 B body)
**Reply:** `RESPONSE.md`, 25,868 B, fetched from page 317 (the send poller timed out at
50 min; `fetch --page 317` archived the reply intact — the round did not need re-sending)
**Pack tip:** `5af57d19` (iter-2762)

Every finding below was re-checked against the tree at `5af57d19` **before** being
adopted. The reply is a set of hypotheses about the artefacts, not a verdict.

---

## Part 1 — the four rulings, as recommended

The reviewer recommended one option per ruling, with a four-part contract and a stated
confidence. Each recommendation below is recorded with **the pack's own evidence**, and
where the reviewer's recommendation **differs from the campaign's prior recommendation**,
that difference is called out — that is the whole point of the round.

### R1 — RT-543: `operant acp` disposition

| | |
|---|---|
| **Recommended** | **(a) deprecate the ACP socket, keep stdio** |
| **Confidence** | Medium overall; High on deprecating `--socket`, Medium between (a) and (b) |
| **Reasoning** | The row's original premise is false (E2); ACP cannot carry an interactive session (E5, RT-545 closed); keeping (c) leaves RT-544 and RT-545 pointing at an undefined daemon split and violates the genome's one-owner law; (b) deletes the daemon before usage is known |
| **Agrees with the campaign's prior recommendation?** | **YES.** The prior plan (2026-11-19 post-2668, A1b) already recorded (a) |
| **Reasoning gap** | Real-world ACP usage is `[UNCONFIRMED]` (E8): no telemetry, no in-tree caller. The one measurement that would settle it: audit editor/launcher configs, or add a startup invocation counter, to see whether anything invokes `operant acp` over stdio or `--socket` |
| **What would move it** | Zero confirmed stdio callers → (b) with High confidence. Active `--socket` consumers → revisit (c) or build an ACP-to-session-socket shim |
| **Law survival** | Survives. Requires: close RT-543 as done with evidence (genome law 4), crate-scoped tests on `operant-cli`, a final `--workspace` check, and a release deploy (it edits `cmd_acp.rs`, so it is **not** docs-only) |

### R2 — RT-280: restart-resume and `/stop` auto-reset

| | |
|---|---|
| **Recommended** | **(c) delete both flags and their three writers** |
| **Confidence** | Low between (c) and (a); **High against (b)** |
| **Reasoning** | Both flags are persisted, read on every access, and written by no production path (E3); the silent-SQLite half is already closed (RT-394), so this is purely a product question; (b) silently changes what a stopped chat retains across 7 platforms with no evidence of demand; **(a) cannot be implemented purely from the pack** — `max_age_seconds` has no answer in the repository |
| **AGREES WITH THE CAMPAIGN?** | **NO — this is the one reversal.** The prior plan recommended (a) "wire `resume_pending` only, delete `suspended`". The reviewer chose **(c) delete both**, on the ground that (a) requires the one number the repository cannot supply. **This is a genuine disagreement and the operator should see it as one.** |
| **Reasoning gap** | The frequency and cost of restart-interrupted sessions is `[UNCONFIRMED]` (E8), and `max_age_seconds` is unspecified. The one measurement: correlate gateway restart/deploy timestamps against in-flight turn timestamps in SQLite/logs over a representative window |
| **What would move it** | Measured mid-turn restart losses plus an owner-chosen `max_age_seconds` → (a), cleanly. Zero measured disruption → (c) to High |
| **Law survival** | Survives. Avoids editing the contended `gateway_runner.rs`. Requires: a schema-compatibility decision (existing SQLite rows must not fail deserialization when the columns go), preserving RT-394's error-propagation contract, crate-scoped tests, then `--workspace`, push, deploy |

### R3 — RT-190: cross-surface memory recall

| | |
|---|---|
| **Recommended** | **(c) accept two families, close RT-190** |
| **Confidence** | Medium; **High that (a) is wrong** |
| **Reasoning** | (a) violates the stated design preferences (the HTTP surface drops `recall_with_weights` / `retain_doc`) and mixes operant writes into a store shared by other harnesses; both stores are live and populated (E4); the RT-191 hooks are topology-agnostic, so closing with (c) now costs nothing if (b) is wanted later; building (b) now is a feature build on an unmeasured need |
| **AGREES WITH THE CAMPAIGN?** | **YES.** The prior recommendation was (c) |
| **Reasoning gap** | Whether the operator actually feels pain from disconnected banks is `[UNCONFIRMED]` (E8). The one measurement: audit a sample of failed/incomplete recall lookups on both sides and count how often the missed fact lived in the other database |
| **What would move it** | Observed repeated cross-harness misses → (b). Zero observed misses → (c) to High |
| **Law survival** | Survives cleanly, **docs/ledger-only** — exempt from the deployment law. Only `docs/audit/2026-06-19-BUGS.md` changes, plus the push |

### R4 — ASKS Q4: enhanced-keys probing

| | |
|---|---|
| **Recommended** | **(b) accept the gap, close Q4** |
| **Confidence** | Medium; **High against (c)** (keeping it open) |
| **Reasoning** | Three fallbacks ship and are unit-tested, one verified on a real PTY with no Kitty bytes (E6); the cheap alternatives are blocked (codex's probe is fork-only; runtime inference from `\r` is impossible because crossterm drops the flags); (a) adds ~120 lines of timing-sensitive raw `/dev/tty` I/O to the TUI input path for a narrow benefit; (c) keeps the last `genome lint` warning open |
| **AGREES WITH THE CAMPAIGN?** | **YES.** The prior recommendation was against building (a) |
| **Reasoning gap** | Whether users actually hit Shift+Enter on non-enhanced terminals and miss the tip is `[UNCONFIRMED]` (E8). The one measurement: observe how often a user sends an unintended single-line prompt or abandons multi-line input in a non-enhanced terminal |
| **What would move it** | Evidence of frequent accidental sends → (a), or a **zero-probe static footer hint** (the reviewer's named alternative). Zero complaints → (b) to High |
| **Law survival** | Survives cleanly, **docs/ledger-only**. Clears W014 without weakening any gate, and avoids editing the contended TUI input files |

## Part 2 — the ranking (cost-of-being-wrong × difficulty-of-undo)

The reviewer ranked **R2 > R1 > R4 > R3**. Adopted, with the reversibility split made
explicit because that is the advice:

| rank | ruling | irreversible option | reversible / deferrable recommendation |
|---|---|---|---|
| 1 | **R2** | (b) irreversibly changes what stopped chats retain across 7 platforms; (c) removes persisted columns (recoverable by git, but needs schema care) | — |
| 2 | **R1** | (b) deletes a daemon and breaks external ACP stdio editors | **(a)** a warning line + doc edits, reversible either way |
| 3 | **R4** | (a) can silently swallow early keystrokes | **(b)** a ledger closure, reopenable anytime |
| 4 | **R3** | only (a), by losing in-process recall | **(c)** a config boundary, deferrable indefinitely |

**The practical consequence:** R3 and R4 are **reversible and can be deferred
indefinitely**; R2 and R1 carry the irreversible options, and R2 is the one where the
irreversible option is not the recommendation — the operator is choosing between two
reversible paths.

## Part 3 — the keystone

The reviewer reports, and the tree confirms, that **none of R1–R4 mechanically unties
another** — they govern four disjoint surfaces (`cmd_acp.rs`, `gateway_session.rs`,
`memory_wire.rs`, the TUI input path).

Across the wider ledger, **R1 is the keystone**: it closes RT-543, resolves the owner
pointer held by the already-closed RT-544 and RT-545, unblocks canonicalizing
`docs/library/cli-surface.md` and `docs/library/session-server.md`, and makes
`operant gateway run` the sole canonical interactive daemon — which is what makes R2's
session-lifecycle rules apply to an uncontested owner.

**Tactical note (adopted):** R4 alone clears the last `genome lint` warning (W014);
R1 + R2 + R3 together clear all three open ledger rows.

## Part 4 — findings about MY OWN PACK (all verified, all fixed)

The most valuable output of the round was not the advice — it was the audit of the pack.
Every item below was re-checked against the tree and **is real**; each was corrected in
the round's files rather than left for a later reader.

| # | finding | verified? | correction |
|---|---|---|---|
| 1 | STATE.md titled the gate battery `iter-2761` while citing `iter-2762`'s supply-chain row | **YES** — `5af57d19` *is* iter-2762 | retitled to iter-2762 |
| 2 | `~2,116` pub fns carried no deriving command and no evidence class, violating the project's own RG-016 | **YES** — the script prints `2116`, so the figure is derivable and my tilde was noise | stated as `2,116` with the deriving command and `[CERT]` |
| 3 | The lib-suite figure was measured at `6a90b5c4`, not at the pack tip | **YES** | now says "(an earlier tip; not re-run at `5af57d19`)" |
| 4 | "In the main tree those 5 pass" was tagged `[CERT]` citing fleet rule 6 — a **policy note**, not a gate run | **YES** | retagged `[PRIOR]`, with the distinction named |
| 5 | The records count said "21 RG records" and omitted the 1 retired | **YES** — `genome doctor` says "44 AD + 21 RG active, 1 retired" | now "21 RG records, plus 1 retired" |
| 6 | ASK.md said "the one red gate left" while STATE's own table shows **two** gates at exit 1 | **YES** — supply-chain *and* pub-fn | ASK.md now names both, and says which five are green |
| 7 | "No code row is open" overstates it: R2 needs code and R3's (b) is a feature build | **YES** | reworded: "no row is open for want of a *fix* — what is absent is a ruling" |
| 8 | "4.6× more rows" was a **file-size** ratio presented as a row count | **YES** — 23,658,496 / 5,169,152 is bytes; a SQLite file carries indexes, FTS/vector tables and free pages | now "4.58x the bytes … NOT a row count" |

**The transferable lesson:** seven of the eight defects were in *my own* evidence
handling — a stale tip reference, an underived figure, a mis-tagged evidence class, a
count copied from prose rather than derived. The reply caught them by re-deriving what
the pack asserted. This is exactly what the round is for, and it is why the pack's own
rule is that a figure without a deriving command is a claim.

## Part 5 — the reviewer's hidden-decision findings (lens L4)

The reviewer named four silent decisions beyond RT-280. All four verified against the
tree:

1. **ACP `session/cancel` acknowledges without interrupting** (`acp/mod.rs`). A client
   issuing a cancel receives an apparent success while execution continues. Making it
   explicit is a docs-only edit (near-zero cost) or an unsupported/error response.
2. **Enhanced-keys fallback with no startup detection** — the `/terminal-setup` tip only
   fires once a user is *already* using a fallback. R4's (b) makes this explicit in
   docs/help text at near-zero cost.
3. **Config-stated memory isolation with an ambiguous provider reference** — this is the
   one I verified and it is **real and actionable**: `AGENTS.md`'s Architecture Overview
   tree (lines 862, 860) still reads `memory_provider.rs # memory-wire + BuiltinProvider`
   and `memory_tools.rs # memory_* tools (memory-wire/builtin)`, while the same file's
   Design Preferences say twice that `BuiltinProvider` **was REMOVED** (lines 636-638 and
   1574-1575), and the code agrees (`memory_provider.rs:366`: "BuiltinProvider is
   deleted"). **AGENTS.md contradicts itself and no gate catches it** —
   `check-doc-claims.sh` verifies that tree *paths* exist, not that their trailing
   comments are true.
4. **Two red gates tolerated as normal** (the wasmtime advisories and the pub-fn queue)
   via fleet rule 7. The reviewer's point: the project is pushing to `origin/main` past
   two permanently red gates without deciding whether `plugins-wasm` (extism, which pins
   wasmtime 49.0.3) should stay enabled. **This is a real gap and it is the strongest
   finding in the reply**, because it is the only one that names something no existing
   plan owns.

## Part 6 — the premise audit (lens L7)

The reviewer tested the request's own framing and returned:

> **The remaining open rulings are product decisions, not missing architecture or active
> architectural bottlenecks.**

Verified. The real architectural work behind all four is already shipped and pinned:
socket sharing (`rt543_one_socket_shares_one_mcp_child_set.rs`), the ACP protocol's
limits (RT-545, closed), SQLite error propagation (RT-394, closed), topology-agnostic
memory hooks (RT-191, closed), and PTY-verified multi-line TUI fallbacks (E6). What
remains is one live integration gap (RT-280's dead flags), one deliberate redundancy
(`AcpRuntime` in `cmd_acp.rs`), and one bounded config fragmentation (two SQLite stores) —
**none of which requires architectural invention to unblock.**

That is the round's headline: **the project is at a decision boundary, not an
architecture boundary.** The next session's work is four rulings plus two maintenance
queues (the pub-fn acid tests and the wasmtime triage).

---

## What this round leaves behind

- **Four recommendations**, each with a confidence and a named measurement that would
  change it (Part 1).
- **One reversal** to flag for the operator: R2, where the reviewer chose (c) delete-both
  against the campaign's prior (a) wire-`resume_pending` (Part 1, R2).
- **Eight verified corrections to the pack itself** (Part 4).
- **Four hidden decisions**, of which #3 (the AGENTS.md self-contradiction on
  `BuiltinProvider`) and #4 (two tolerated red gates) are new and actionable
  (Part 5).
- **The premise verdict**: decisions, not architecture (Part 6).

## What still needs the operator

Every one of R1–R4. The reviewer's role was to recommend; the ruling is the owner's. The
pack gives the operator a per-row recommendation, its confidence, and the one measurement
that would change it — which is the most an external review can hand over.
