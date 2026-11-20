# PACK-AS-SENT — RECORD, not a reconstruction

The reviewer read this pack inline as a **single message of 46,763 B** on
2026-11-20 at 20:02 (`SEND.log`).

The exact bytes sent are **NOT preserved here.** `PACK-AS-SENT.md` was twice
overwritten by a `context_pack.py build` run *after* the send (the second one was
a mistake in the same shape as the first). That is recorded and explained in
`CORRECTION.md`.

## The pack that was sent, by section

| section | bytes at send |
|---|---|
| `C-ASK` | 12,393 |
| `B-STATE` | 6,173 |
| `E-EVIDENCE` | 7,315 |
| `D-LAW` | 5,527 |
| `F-LENSES` | 5,765 |
| `G-CONTEXT` | 3,277 |
| **total** | **41,351** |

## What is reconstructable

The six source files (`ASK.md`, `STATE.md`, `EVIDENCE.md`, `LAW.md`, `lenses.md`,
`G-CONTEXT` section of `STATE.md`) plus `pack.spec` reproduce the *structure* and the
*sections*, and each source file carries its own dated corrections. What they do not
reproduce is the exact whitespace and the exact bytes as sent.

`CORRECTION.md` lists every content change between the sent pack and the corrected
sources, with the section-level byte deltas.

## The next round's obligation

Preserve the as-sent bytes **before** any rebuild. Two ways, both sufficient:

1. `cp PACK-AS-SENT.md PACK-AS-SENT-at-send.md` immediately after the build that was
   sent, or
2. record the SHA-256 of the sent file in `DISPATCH.json` and treat a mismatch as drift.

Either way, the drift gate must compare the sources against the **preserved sent copy**,
never against a rebuilt one, and it must not exempt itself on `SEND.log` alone.
