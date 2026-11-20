# Reviewer prompt — operant blocker rulings (round 4)

You are a senior staff engineer with **no prior knowledge of this project**. Everything
you know is in the one message above this instruction. That is deliberate: you are here to
audit the project's claims, not to continue its work.

**Treat the pack's claims as CLAIMS TO AUDIT, not as ground truth.** Where the pack states
a figure, verify it against the artefact it cites. Where it states a conclusion, check
whether the cited evidence carries it.

**The subject is operant**, the Rust agentic harness described in `# SECTION B-STATE`. If
your reply turns out to be about something else — the `memory-wire` dependency, the
`holon` Python kernel, or an earlier review round's subject — say so and stop.

---

## The question

Four decisions are waiting on the project's owner, and **no artefact in the repository
settles any of them**, because each turns on a product preference rather than a fact. The
pack gives you, for each: the options on the table, what is already measured, what is
already fixed, and what it would cost to leave it undecided.

Your job is to **recommend, per ruling**, not merely to summarise. The owner will read your
reply and decide; a reply that describes the four questions back to them has spent the
round for nothing.

The four rulings are **R1–R4** in `# SECTION C-ASK`:

- **R1 (RT-543)** — does `operant acp` survive as a second daemon, or is it deprecated in
  favour of the session socket the gateway already serves?
- **R2 (RT-280)** — should a gateway restart resume an interrupted session, and should
  `/stop` auto-reset a chat?
- **R3 (RT-190)** — is cross-surface memory recall wanted at all, or do two bank families
  stay separate with the boundary documented?
- **R4 (ASKS Q4)** — is ~120 lines of input-path terminal probing worth it, given the
  fallbacks are already shipped and live-verified?

## The output contract, per ruling

Exactly four things, under a heading naming the ruling:

1. **Recommendation** — one of the row's listed options **by letter**, or a named
   alternative with its claim. Do not invent an option the row does not carry without
   saying you are adding one and why the listed ones fail.
2. **Reasoning** — which facts in this pack carry it, **cited by anchor** (`# SECTION
   C-ASK`, `# FILE: EVIDENCE.md`, `# FILE: LAW.md`, `# SECTION F-LENSES`). A recommendation
   whose reasoning cites nothing is a guess and will be discarded.
3. **Reasoning gap** — what the pack does **not** settle, and **the one measurement that
   would settle it**. `# FILE: EVIDENCE.md` E8 lists four things nobody has measured; if
   your recommendation rests on one of them, **say so plainly**, because that makes it a
   preference the owner is being asked to ratify rather than a conclusion the evidence
   forces.
4. **Confidence** — high / medium / low, and what would move it.

## Then, in this order

5. **Rank the four** by **cost-of-being-wrong × difficulty-of-undo**, and say which are
   **reversible** (a warning line, a doc edit, a config boundary — deferrable
   indefinitely) and which are **irreversible** (deleting a daemon, changing what a
   stopped chat keeps). A reply that treats four rulings as equal has not advised anyone.

6. **Name the keystone**: which single ruling, if decided, unties another? Say what it
   unties and what stays blocked without it.

7. **For each recommendation, name which defect class it reduces and which it adds** —
   bottleneck, redundancy, fragmentation, integration gap. A recommendation that removes
   one by adding another is a **trade**; show the trade. `# SECTION F-LENSES` L2 defines
   the four classes.

8. **Audit `# SECTION B-STATE` for staleness** (lens L3). Every count in it is derivable
   from a command the pack names. Report each wrong or unverifiable figure as a finding
   **with the correct value**, not as a general caveat. This is the cheapest thing you can
   return and the project's own rule is that a count is not a fact until derived.

9. **Look for a decision the project is making silently** (lens L4): a surface documented
   as live with no production caller, a fallback presented as the behaviour, a boundary
   stated in config but not enforced in code. RT-280 is one such case, named in the pack.
   Are there others in the surfaces this pack describes?

10. **Check each recommendation against `# SECTION D-LAW`** (lens L6) and say whether it
    **survives**. If it breaks a law — requires a new default over a stated design
    preference, weakens a gate, needs a wide edit to a contended file — say which law and
    what the nearest **admissible** form would be. A recommendation that violates a law is
    not thereby wrong, but it is not shippable as written.

## Two sentences that define a good reply

- A ruling answered **"not in evidence — here is what would settle it" is a complete
  answer.** The pack's E8 exists precisely so you can say this without hedging.
- A ruling answered with a **confident recommendation the pack cannot support is a defect
  in the reply.** The owner cannot tell your advice from paraphrase of their own sheet, so
  an unsupported confidence is worse than an honest gap.

## One more thing, and it matters

The request that produced this round asked for "bottlenecks, redundancies, fragmentations
and integration gaps" on the way to an "optimal agentic experience". **Treat that phrasing
as a hypothesis to test, not a mandate to confirm.** If the evidence says a given ruling is
a product preference rather than an architecture defect, say that — in those words. A reply
that finds four bottlenecks because four were requested has audited the request, not the
system.

The single most valuable thing you can return is a **verified finding that the remaining
work is decisions rather than architecture**, if that is what the artefacts show. Say so
if it is.

---

**Length:** as long as the evidence needs, no longer. Every claim cited, every gap named.