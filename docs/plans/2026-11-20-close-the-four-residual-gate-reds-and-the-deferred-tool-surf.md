# Close the four residual gate reds and the deferred tool-surface tranche

**Date:** 2026-11-20
**Status:** PLANNED (frozen on mint)
**Predecessor:** `2026-11-20-close-the-remaining-consolidation-gaps-the-config-surface-ra.md` (iters 2717-2727, executed)

## Why this plan exists

The predecessor closed the config-key ratchet (31 → 0) and the supply advisories. It
left three gates red and one tranche unstarted. This plan re-derives all four from the
tree at `26ee7218` rather than trusting the predecessor's prose — and one of the four
turned out to be a **gate bug, not a doc bug**, which is the same class as iter-2719's
over-capture.

Gate battery measured at `26ee7218`:

| gate | exit | verdict |
|---|---|---|
| `scripts/audit-config-keys.py --check` | 0 | green (predecessor) |
| `scripts/audit-ledger-index.py` | 0 | green (predecessor) |
| `bash scripts/supply-chain-gate.sh` | 0 | green (predecessor) |
| `scripts/audit-pub-fn-surface.py --check` | 1 | **142 unaccounted candidates** |
| `bash scripts/check-doc-claims.sh` | 1 | **2 drifts** |
| `./scripts/genome lint` | 2 | **E019 x2 + E104** |

## The items, in measured cost order

### T1 — `check-doc-claims.sh` counts a COMMENT WORD as a tool name (gate bug, Severity 2)

**The claim.** `docs/library/README.md` says "the 67 names in `builtin_tool_names()`".
The gate derives **68** and reports a drift.

**What is actually true.** The literal array in
`crates/operant-core/src/tools/builtin.rs:328` holds **67** entries, verified two ways:
`rg -c '^\s+"[a-z_0-9]+",'` over the literal → 67, and `builtin.rs:407` is
`"ui_request_input"`, the 67th. The 68th name the gate counts is **`notification`**,
which appears at `builtin.rs:354` inside a *comment* explaining why the entry is `notify`
and not `notification`:

```
// (iter-2716, Hydrogen round-2 F3) `notify`, not `notification`: the
// (tools/notification_tool.rs:26), so the old literal here made
// `operant tools disable notification` write a name the runtime never
// matches -- a silent no-op. `classify.rs` keeps "notification" as a
```

The gate's regex is `r'"([a-z0-9_]+)"'` applied to the whole function body, comments
included. **The doc is right; the gate is wrong.**

**Why this matters beyond a red gate.** This is the *same defect class* as iter-2719,
which fixed the config auditor's struct regex over-capturing. A gate that miscounts its
own subject produces a false drift on every run, and a false drift that is "fixed" by
editing the doc corrupts the doc. Two agents could "resolve" this in opposite directions.

**The fix.** Strip `//` line comments and `/* */` blocks from the function body before
extracting names — not by loosening the regex, which would then under-count a real
multi-line entry.

**Verify:** the gate exits 0 with `docs/library/README.md` untouched; a mutation that
adds a real 68th tool name makes it exit 1 again (proven both ways); a mutation that
adds a comment mentioning a quoted name does NOT trip it.

### T2 — AGENTS.md says 29 TUI overlays; the array holds 32 (doc drift, Severity 1)

**The claim.** `AGENTS.md` (three places, per `check-doc-claims.sh`) says
`overlay_flags()` names **29**.

**What is actually true.** `crates/operant-cli/src/tui/app/commands.rs:93` is
`pub(crate) fn overlay_flags(&self) -> [(&'static str, bool); 32]`. The array holds 32
unique names. `git show 95fc927a^` holds 29; the three added are
**`agent_picker`**, **`context_menu`**, **`subagent_navigator`** — peer work
(iter-2714, TUI audit round), landed without the prose being re-derived.

**The fix.** Update the count to 32 **and** name the three new overlays, so the next
reader can tell which entries are recent. Do NOT re-derive the number by hand — the
whole point of the gate is that it derives; the prose follows the gate's output.

**Verify:** `check-doc-claims.sh` and `genome lint` both exit 0 on the overlays row.

### T3 — `genome outline` zone is stale (E104, Severity 1)

**The claim.** `AGENTS.md#stack` no longer matches `genome outline`.

**The fix.** `./scripts/genome outline` regenerates the zone. This is generated output —
never hand-edit it. (iter-2723 did exactly this and a peer's push re-staled it within the
hour, so this item recurs; the durable fix is to make the zone a check the gate runs, not
a thing an agent remembers to regenerate.)

**Verify:** `genome lint` exits 0.

### T4 — `output_was_cut` detects terminal truncation by string-sniffing (Severity 2)

**The claim (hydrogen round-2 F9).** `crates/operant-core/src/tools/terminal_tool.rs:36`
decides whether output was cut by testing `text.ends_with(marker)` against two literal
strings — `"[output truncated]"` and `"...[truncated]"`.

**Why it is wrong, concretely.** The function infers a property of the *envelope* from
the *content*:

- A command that legitimately prints `[output truncated]` as its last line — a `tail` of
  a log that contains that literal, a test fixture, a build script echoing a marker — is
  reported as "stdout was cut at the 200000-byte output cap". That is a false claim
  delivered to the model about its own command's output.
- The inverse is worse: the cut is decided by *this* function, so the only thing that
  can set the marker is the backend that already knows it cut. The information exists
  upstream and is being thrown away and reconstructed from text.

**The fix.** The backends that cap must **return** the fact. Add a `truncated:
Option<Truncation>` (which stream, what cap) to the backend's output type, set at the
site that does the cut, and let `terminal_tool.rs` read it. Delete `output_was_cut` and
`TRUNCATION_MARKERS`.

**Verify:** a command that prints `[output truncated]` as its own last line reports
`truncated: None` (this is the regression test — it fails against today's code and
passes after); a command that exceeds the cap still reports the cut, with the right
stream and byte count.

### T5 — 142 unaccounted `pub fn` candidates (surface, Severity 3)

**The state.** `scripts/audit-pub-fn-surface.py --check` reports 142 candidates with no
caller. 58 have `tests=0`.

**Why this is not one tranche.** Each candidate needs its own acid test (remove `pub`,
`cargo check -p operant-core --lib`, read the verdict) because the compiler cannot see
this class at all — a `pub fn` on a `pub` type in a library emits no `dead_code` warning.
The predecessor learned this the hard way: four `GuardScanner` methods looked dead by the
acid test but were the only public seam to *private* free functions, and deleting them
removed a real API boundary. The acid test is necessary and **not** sufficient — the
delegate's own reachability has to be checked too.

**The honest disposition:** 142 is too many for one tranche and most are probably fine.
The productive subset is the `tests=0` group (58), taken in batches of ~6 with the
delegate-reachability check applied to each. Anything that survives the acid test *and*
has no caller goes into `ACCOUNTED_FOR` with the honest reason, or gets deleted.

**Verify per batch:** gate exit unchanged or improved (never worse — the predecessor
confirmed 142 before and after its own change), and each deletion carries a
mutation-proven test.

## Explicitly NOT in this plan

- **RT-190** (memory-wire two-bank fragmentation) — ruled option (c), documentation
  closed iter-1373; the bank-topology decision is the operator's, not an agent's.
- **RT-280** (gateway resume-after-restart never switched on) — a design decision about
  whether a session id survives a restart. Owner-gated.
- **RT-543** (two daemons) — premise corrected 2026-11-20; the row stays open for the
  ACP disposition, which is an owner call.
- **112 inherited config-key gaps** in pre-existing keys — reported by the gate,
  explicitly not this change's to fix.
- **Peer-owned red gates.** If a gate is red from a peer's in-flight subsystem, prove the
  delta adds zero and say so (fleet rule 7) — do not fix another slice's work mid-flight.

## Order of work

```
1. T1  (gate bug, unblocks T2's measurement)   → verify: gate 0, doc untouched
2. T2  (doc drift, the number T1 confirms)     → verify: doc-claims 0
3. T3  (regenerate the zone)                   → verify: genome lint 0
4. T4  (real bug, real code change)            → verify: mutation-proven test
5. T5  (batches, only as budget allows)        → verify: gate never worse
```

T1 before T2 deliberately: T2's correct number is exactly what T1's fixed gate derives,
so doing T2 first would mean writing a number from a broken measurement.

## Definition of done

- `check-doc-claims.sh`, `genome lint`, `audit-config-keys.py --check`,
  `audit-ledger-index.py`, `supply-chain-gate.sh` all exit 0.
- `audit-pub-fn-surface.py --check` is no worse than 142.
- Every code change carries a regression test that fails without it, mutation-proven.
- Every doc number is derived from the gate, never hand-written.
- Each item is its own iteration, committed and pushed to `origin/main` with a unique
  label re-derived after `git fetch origin`.