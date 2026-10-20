<!-- archived 2026-10-20 from docs/regressions/RG-002-a-numbered-record-is-immutable-once-numbered-the-cli-enforce.md (lines 81-103): test whether the tool allows section-level supersession of a numbered record -->

## Detection

Cheap and effective, and it was the finder:

```bash
git log --follow --oneline -- docs/regressions/ docs/decisions/
```

A record touched by more than one commit was amended in place. Every numbered
record should appear exactly once, in the commit that minted it.

`genome lint` cannot do this today (E103/E106 are supersession checks, not
mutation checks), and that gap is the same shape as RG-001's: internally
consistent output that cannot see its own subject. Until the CLI grows a
mutation check, the manual gate above is the only one, which means the
bypass is detectable — but only by someone who thinks to look.

This record is the correction RG-001 needed: it is a new record, minted through
the sanctioned path, superseding nothing because RG-001's IDENTITY stands and
RG-001's law was sound. The two additions RG-001 needed (the citation defect and
the fifth law) are recorded in the iter-1062 body and stay in RG-001's amended
text, with this record naming that they were added without a revision trail.

