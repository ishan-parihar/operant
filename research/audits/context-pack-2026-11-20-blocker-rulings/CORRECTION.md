# CORRECTION — what changed between the pack the reviewer saw and the pack on disk

**This round's `PACK-AS-SENT.md` was REBUILT after the send.** It is therefore a
post-send reconstruction, not the exact bytes Hydrogen read.

## What the reviewer actually saw

| | |
|---|---|
| sent at | 2026-11-20 20:02 (`SEND.log`) |
| sent body | **46,763 B** (inline, sentinel last) |
| pack at send | **41,351 B** (`C-ASK` 12,393 · `B-STATE` 6,173 · `E-EVIDENCE` 7,315 · `D-LAW` 5,527 · `F-LENSES` 5,765 · `G-CONTEXT` 3,277) |

## What is on disk now

**`PACK-AS-SENT.md` is no longer a rebuilt blob.** It is a record (see below).

The corrected sources total **42,826 B** — the same sections, rebuilt at 21:13 after the
verification pass corrected the pack's own defects.

**This happened twice.** The first rebuild (21:06) was deliberate; a second rebuild
(21:13) was a mistake of the same shape — I ran the build again to "capture the latest
corrections" without preserving the first. Two lessons, one of them redundant:

- the as-sent bytes are gone, so `PACK-AS-SENT.md` is now a **record** explaining what was
  sent (46,763 B inline body; 41,351 B of pack), not a reconstruction;
- **a documented mistake is not a prevented one.** Writing the warning above did not stop
  me repeating it twenty minutes later. The prevention has to be mechanical: preserve the
  blob or record its hash *before* the rebuild, and make the drift gate compare against
  that preserved copy.

## Exactly what changed, so the two are reconcilable

All changes were to the **source files**, and every one is recorded in
`VERIFICATION.md` Part 4 as a verified finding:

| section | sent | on disk | why |
|---|---|---|---|
| `C-ASK` | 12,393 | 12,418 | "the one red gate left is the supply-chain gate" was FALSE — two gates exit 1; now names both and says which five are green |
| `B-STATE` | 6,173 | 6,505 | the iter-2761/2762 mislabel; the underived `~2,116`; the `[CERT]`-on-a-policy-note tag; the missing retired record |
| `E-EVIDENCE` | 7,315 | 7,696 | "4.6x more rows" was a file-size ratio, not a row count |

`D-LAW`, `F-LENSES` and `G-CONTEXT` are **byte-identical** to what was sent.

## Why this happened, and why it matters

The protocol says `PACK-AS-SENT.md` is "a record of what the reviewer saw, never a living
doc". I rebuilt it after the send to keep the corrections in the committed artefact, and
in doing so overwrote the record. The corrections are right; the **recording** was wrong.

**The lesson, named so it is not repeated:** when a verification pass corrects a pack's
own defects, the sources are edited and the pack is rebuilt — but the *as-sent* blob must
be preserved (copied to `PACK-AS-SENT-at-send.md` or a checksum recorded) **before** the
rebuild, and the drift gate should then compare the sources against the *sent* copy rather
than silently exempting the round on `SEND.log`.

## Consequence for this round

None of the four rulings or the verification findings are affected: the reviewer's reply
audited the sent bytes, and `VERIFICATION.md` records each finding against the tree at
`5af57d19` rather than against either blob. The affected claim is only the pack's internal
consistency, which the verification pass already repaired.

The next round starts from `pack.spec` (unchanged as a spec) plus these corrected
sources, and should preserve its as-sent blob.