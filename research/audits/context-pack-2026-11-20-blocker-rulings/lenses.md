# LENSES — the instruments this round must apply

Each lens is a question the reply must answer **with a finding**, not a summary. A lens
that produces no finding should say so explicitly ("this lens found nothing at this
surface") — that is a result, not a gap.

---

## L1 — The ruling lens (primary)

For **each** of R1–R4, produce the four-part output contract:

1. **Recommendation** — one of the row's listed options (by letter), or a named
   alternative with its claim.
2. **Reasoning** — which facts in this pack carry it, **cited by anchor**
   (`# SECTION ASK`, `# FILE: EVIDENCE.md`, `# FILE: LAW.md`). A recommendation whose
   reasoning cites nothing is a guess.
3. **Reasoning gap** — what the pack does **not** settle, and **the one measurement that
   would settle it**. If the recommendation rests on an `[UNCONFIRMED]` row from E8, say
   so plainly: that is a preference, and the operator should know which preferences they
   are being asked to ratify.
4. **Confidence** — high / medium / low, and what would move it.

## L2 — The defect-class lens

For each recommendation, name **which defect class it reduces and which it adds**. The
four classes this round is about:

- **bottleneck** — a binding constraint on throughput, iteration speed, or correctness
- **redundancy** — two implementations of one thing
- **fragmentation** — two halves that should be one
- **integration gap** — wired but unreachable, or reachable but unwired

A recommendation that removes a bottleneck by **adding a seam** is a trade, and the
operator should see the trade rather than the slogan. A recommendation that claims to
reduce all four with no cost is not credible.

## L3 — The staleness lens

Audit **STATE.md** against the artefacts it cites. Specifically:

- Does the gate table match what the gates actually report at this tip?
- Is any count in STATE.md derivable and wrong? (7 crates, 53 CLI variants, 7 platforms,
  3 open ledger rows, 659 closed, 2,116 pub fns.)
- Does any sentence in STATE.md claim something the pack's own evidence contradicts?

Report each discrepancy as a **finding with the correct value**, not as a general "the
state doc may be stale". A stale figure is the single cheapest thing this round can
return, and the project's own rule (RG-016) is that a count is not a fact until derived.

## L4 — The hidden-decision lens

R1–R4 are the decisions the project *knows* it is waiting on. Look for a decision it is
**making silently** by not deciding:

- A surface that is documented as live but has no production caller (RT-280 is one; are
  there others in the surfaces named here?)
- A fallback presented as the behaviour rather than as a fallback
- A boundary stated in config but not enforced in code

Name it, cite where it hides, and say what it would cost to make it explicit.

## L5 — The untying lens

Which of R1–R4, if decided, **unties another**? Which is the keystone? A ruling that
closes one row and unblocks three is worth more than one that closes one row and unblocks
none, and the operator's scarcest resource is attention — so rank the four by
**cost-of-being-wrong × difficulty-of-undo**.

Explicitly name the ones that are **reversible** (a warning line, a doc edit, a config
boundary) and can therefore be deferred indefinitely, versus the ones that are
**irreversible** (deleting a daemon, changing what a stopped chat keeps) and cannot.

## L6 — The law-survival lens

For each recommendation, check it against LAW.md and say whether it survives:

- Does it require a **new default** that overrides a design preference? (R3 option (a) does.)
- Does it require **weakening a gate** or allowlisting a finding? (forbidden by the
  standards)
- Does it require a **wide edit to a contended file** (gateway_runner.rs, the TUI) whose
  coordination cost the fleet rules price?
- Does it name a **deploy step**, or is it docs-only?

A recommendation that violates a law is not thereby wrong — it is **inadmissible as
written**, and the reply should say which law it breaks and what the nearest admissible
form would be.

## L7 — The anti-slogan lens

The request this round came from says the goal is to "address and operationalize the
optimal-agentic-experience" by finding "bottlenecks, redundancies, fragmentations, and
integration gaps". **Treat that phrasing as a hypothesis to test, not a mandate to
confirm.**

If the honest answer for a given ruling is "this is not a bottleneck; it is a product
preference", say that. A reply that finds four bottlenecks because four were requested has
audited the request, not the system. The most valuable possible output of this round is a
**verified finding that the remaining work is decisions, not architecture** — if that is
what the evidence says.

---

## The subject check

The subject is **operant**, the Rust agentic harness described in STATE.md. The plausible
wrong subjects are: the `memory-wire` project (a *dependency*, not the subject), the
`holon` Python kernel (a *separate* project that proposes to use operant as its
runtime), and the prior review rounds' subjects (round 1 and 2's architecture surfaces).

**If your reply is about any of those rather than operant, say so and stop.** A
wrong-subject verdict reads as a rejection of the architecture, and every finding in it
comes back "not in evidence" against this pack's artefacts.

## The two sentences that make this round honest

- A ruling answered **"not in evidence — here is what would settle it" is a complete
  answer.**
- A ruling answered with a confident recommendation **the pack cannot support is a defect
  in the reply.**