# Plan: Recursive Self-Observability for the Operant Runtime

- **Date:** 2026-11-03
- **Author:** jcode
- **Trigger:** the operator's 2026-11-02 report (input-box text leak + Shift+Enter dead) plus an
  agent-authored proposal listing recursive-debugging infrastructure, read against the tree.
- **Status:** pre-registered, frozen. Implementation iterates against this file; the file does not
  move.

---

## 0. What this plan is NOT

The agent proposal that triggered this plan describes its own ideal: a turn-level trace recorder,
step-level replay engine, interactive breakpoints, a trajectory DAG, a side-effect ledger, an
A/B experiment harness, a blast-radius limiter, "a sandbox for self-modification", and a
segregation layer with write gates over three data planes.

Measured against this tree, **most of that is already built or already not-worth-building**.
The plan below records what is real and what is actually missing, because building from the
proposal's list would spend a week re-implementing `operant trajectory`, `operant insights`,
`operant context`, `operant checkpoints`, `operant debug`, the 24-class error classifier and the
whole `tui debug simulate` harness.

| Proposal item | State in this repo | Verdict |
|---|---|---|
| Trajectory recorder | `crates/operant-core/src/trajectory.rs` + `operant trajectory list/show/export` | **built**, but blind (see G-1) |
| Session archive with provenance | `operant-core/src/session/` + `sessions` + the new session hub (iter-1671/1673) | built |
| Cross-session search | `session_search` / `memory_search` / `ctx_search` surfaces + `memory-wire` | built |
| Advisor probe log collector | `crates/operant-core/src/advisor.rs` (loop guard, emission guard, quarantine) | built inside core; **not surfaced** (G-4) |
| Failure taxonomy | `agent/error_classifier.rs`, 24 classes (`FailoverReason`) | built, **per-turn only** (G-3) |
| Tool health dashboard | `operant tools`, `operant doctor`, MCP management | built |
| Context plane inspector | `operant context` (lossless DAG) + footer gauge | built |
| Config drift detector | `operant config`, `dump`, `checkpoints` | partially (G-5) |
| Turn-level trace + latency | `agent/turn_attribution.rs` (4 spans, iter-1669) | **built for the turn**; not persisted (G-1) |
| Step-level replay | — | **missing, and not worth building** (see §4) |
| Interactive breakpoint | — | **rejected** (see §4) |
| A/B experiment harness | — | rejected (see §4) |
| Segregation layer + write gates | approval allowlist + write gates exist | rejected as a build (see §4) |

The two live defects that motivated the report are fixed separately (P0 below); this plan is the
follow-on.

---

## P0 — the two defects the operator reported (SHIPPED this date, iter numbered at push)

1. **Input-box text leak on a failed turn.** Fixed by removing the failed submission's
   transcript row when the prompt is restored
   (`App::drop_transcript_user_row_matching`, called from
   `restore_failed_input_to_composer`, `crates/operant-cli/src/tui/app/messaging.rs`). The submit
   path pushes the user message before the turn runs, so a dead turn left the sentence on screen
   twice — once as a transcript row and once back in the composer where it is editable.
   Verified: `operant tui debug simulate --keys 'hello world<enter>'` renders one `hello world`
   row (it rendered two before). 4 regression tests, mutation-proved RED by deleting the call.
2. **Shift+Enter / Alt+Enter / Ctrl+Enter not expressible — and therefore not testable.**
   Root cause was in the debug harness, not the key handler: `parse_generic_chord` in
   `crates/operant-cli/src/cmd_tui_debug.rs` required exactly one character after the `+`, so
   `<shift+enter>` fell through to `None` and the whole token was **typed as its literal
   characters** — eleven keystrokes of `shift+enter` into the composer. No scenario could
   exercise the newline chord, and the operator's report could not be reproduced or ruled out
   from the harness. `chord_key_code` names the non-character keys; the literal-fallback
   contract is preserved for genuinely unknown names (`<ctrl+ab>` still types literally).
   Verified end to end: `ab<shift+enter>cd` renders two composer lines and submits nothing.
   2 regression tests, mutation-proved RED by removing the branch.

**Residual, honest:** the *production* runtime gap behind the operator's Shift+Enter report is
that a terminal which does not honour the kitty keyboard protocol sends a bare `\r` for every
Enter chord. Operant pushes
`DISAMBIGUATE_ESCAPE_CODES | REPORT_EVENT_TYPES | REPORT_ALTERNATE_KEYS`
(`tui/keyboard_enhancement.rs`) and never checks whether the terminal honoured it. The
trailing-backslash continuation and the `/terminal-setup` hint are the current mitigations.
That gap is G-6 below.

---

## The codex contrast, measured feature-by-feature (not file-by-file)

Module *names* differ almost completely (`codex: chat_composer/{draft_state, slash_input,
popup_state, vim_register}` vs `operant: prompt_input/{editing, suggestions, vim_ops,
kill_ring}`), so a filename diff is worthless — it reports 16 "gaps" that are mostly
renames. Mapped by *behaviour*, the composer is at parity or ahead on: word classes and
chords, @-mention grammar with directory chaining, ghost text, history search, vim with
registers, paste-burst detection, selection and copy, multi-line wrapping and the
codex-parity two-column gutter.

The codex behaviours with **no operant counterpart at all**:

| codex module | what it does | operant equivalent | verdict |
|---|---|---|---|
| `terminal_probe.rs` | queries the terminal's actual kitty flags | none — pushes and hopes | **G-6, real gap** |
| `debug_config.rs` | prints every config layer + each key's requirement source | single-file config, no layers | **G-5 part 2, real gap** |
| `chat_composer/reconnect.rs` | restricted input modes (Disconnected / UnavailableThread) that keep drafts editable and still allow local commands | none | **recorded, prioritize low** — operant's failed-turn restore is the closest thing and now also keeps the draft |
| `attachment_state.rs` | typed attachment lifecycle (local / remote / placeholder deletion reconciliation) | `image_paste.rs` + `pinned_images.rs`, placeholders drop with a warning notice | partial |

Everything else the operator might expect from codex is already here.

---

## G-1 — Trajectory steps carry no latency and no error text (P1)

`TrajectoryStep` records `thought / action / action_args / observation / response / success`.
It does not record **how long the step took** or **what the error string was** — `success:
bool` throws away the distinction between "the tool refused a malformed patch", "the index
wasn't ready" and "the network died", which are exactly the three classes the report's own
smoke test hit. The agent runtime already computes the numbers (`RequestLatency`, the
24-class `FailoverReason`) and the turn-attribution machinery already partitions a turn into
four spans (iter-1669); none of it reaches the step record.

**Build:** add `duration_ms: Option<u64>`, `error: Option<String>` and
`error_class: Option<String>` (the classifier's own label) to `TrajectoryStep`; populate from
the agent loop where the tool result already crosses into the recorder. Persist the
turn-attribution spans alongside. Keep it additive and `#[serde(default)]` so existing
trajectory JSON still loads.

**Check:** `cargo test -p operant-core --lib trajectory`; a round-trip test that a
failing tool call survives show/export with its class and its duration; the exported JSON
for a real session carries the four spans.

## G-2 — `record_metric` is a no-op, and the metric emission side is nearly as dead (P1)

Measured precisely (2026-11-03), because the report's framing overstates this:

- `ObserverEvent` (27 variants) **is genuinely emitted** from the agent loop —
  `AgentStart`, `AgentEnd`, `LlmRequest`, `LlmResponse`, `ToolCall`, `TurnComplete`,
  `Error` all fire from `agent/run.rs`. No dead variants.
- `ObserverMetric` (7 variants) has **exactly one** production emission:
  `RequestLatency` at `agent/run.rs:1329`. `ActiveSessions`, `QueueDepth`,
  `TokensUsed`, `HandRunDuration`, `HandFindingsCount` and `HandSuccessRate` are
  constructed **only** in `crates/operant-api/src/observability_traits.rs`'s own
  test module.
- `TurnSummaryObserver::record_metric`
  (`crates/operant-cli/src/main.rs:2750`) is an empty body, so even that one
  real emission is discarded.

So the highest-leverage fix is not "wire up the sink" alone — a sink for six
metrics that are never emitted is theatre. **Build both halves or neither:**
emit `TokensUsed` and `QueueDepth` at the two points the agent loop already has
the numbers, and make the sink accumulate and print. Do not add a new observer
trait, a new sink, or a dependency.

**Check:** `operant debug share` output contains non-zero latency and token
figures for a session that made tool calls; a test that `record_metric` moves
the accumulator; a grep that each eagerly-emitted variant has a non-test
call site.

## G-3 — Failure classification is per-turn, never aggregated (P1)

`error_classifier.rs` classifies to fail over. Nothing **counts** what it classified, so
"the `aft_apply_patch` patch-format failure happened 12 times in a row before I noticed"
is invisible. The report asks for a failure-taxonomy *view*; the classifier is there, the
aggregate is not.

**Build:** a per-session counter keyed `(tool, error_class)` written into the existing session
store, surfaced by `operant insights failures` (a new subcommand of one the report's proposal
listed as "usage insights" — it already exists).

**Check:** a test that three tool failures with the same class produce one row with count 3;
`operant insights failures` renders it.

## G-4 — Advisor notes are never queryable (P2)

`advisor.rs` produces notes of concern with a loop guard, an emission guard and a quarantine.
Nothing stores them where an operator or agent can ask "what did the advisor say last week?".

**Build:** persist quarantined/emitted notes with session id + turn id, read back through
`operant debug advisor` (or fold into `operant debug share`). Keep the in-core guards.

**Check:** a quarantined note appears in the debug report and a normal one does not double-count.

## G-5 — Config drift has no detector, and there is only ONE layer to drift from (P2)

`load_app_config` (`config.rs:2485`) resolves to a single file — explicit path, else the
first of `default_config_paths()` that exists, else defaults. There is no layer stack.
codex has one (`codex_config::ConfigLayerStack`, plus a `debug_config` surface that
prints every layer including disabled ones with each key's requirement source —
`codex-rs/tui/src/debug_config.rs`).

**Consequence for this row:** a drift detector over a one-layer config answers only
"did the file change on disk", which `git` or a checksum already tells you. The
useful question — "which VALUE is winning and from where" — needs the layer model
does not have. So this row has two parts and the second is the real one:

1. snapshot + diff (~100 lines, honest, ships alone if part 2 is deferred);
2. a layer stack: defaults < project < profile < env < explicit, with the winning
   layer reported per key. This is a much larger change than the proposal imagined
   — it touches every consumer that assumes one config — and it is **not** in scope
   for this plan. It is recorded here so nobody mistakes G-5 for a small win.

**Check for part 1:** a snapshot taken before a `config set` and compared after names
  the changed key and both values. **No check exists yet for part 2 — that is the
  signal it is not specced.**

## G-6 — Enhanced keys are requested but never verified (P2)

The production half of the operator's Shift+Enter report. codex probes the terminal
(`tui/src/terminal_probe.rs`: it sends `CSI ? u` and parses the flags that come back) and
its footer then only advertises Shift+Enter when the probe succeeded
(`footer_insert_newline_key`, `chat_composer.rs:4431`). Operant pushes and hopes.

**Build:** mirror codex's probe at startup, store the answer, and let the composer's hint row
and `/terminal-setup` tell the truth ("your terminal does not report enhanced keys, here is
the fix" instead of showing Shift+Enter as available).

**Check:** the hint row differs for a supported vs an unsupported probe answer, driven by an
injected byte stream in a test — the same shape codex's own probe tests use.

---

## Rejected, with reasons (do not re-litigate)

- **Step-level replay engine.** Deterministic replay requires re-dry-running side-effecting
  tools (file writes, web posts, `memory_retain`) under constraints that do not exist here.
  The rewind machinery already gives the useful half: roll the conversation back and edit the
  prompt (iter-1633/1637, corpus-proven). Replay of the *decision* is not replay of the *effect*.
- **Interactive breakpoint / self-pause.** `/stop`, `/pause`, `/resume` and the ESTOP already
  suspend the loop; a breakpoint adds a state no production path exercises and an
  unreachable arm to every tool dispatch.
- **A/B experiment harness.** Two live loop policies diverge on the first non-determinism and
  the comparison never converges. The scenario corpus (`tui debug simulate` + committed
  goldens) is the deterministic half and it is already strong.
- **Segregation layer with explicit read/write planes.** The approval allowlist and write
  gates already sit in front of every mutation; a parallel plane model would be a second,
  weaker copy of the same authority.
- **Blast-radius limiter, no-self-deletion.** The agent does not hold a process that can
  rewrite its own dependency pins at runtime, and these guards defend a capability it does
  not have.

---

## Sequencing

| # | Item | Size | Depends on |
|---|---|---|---|
| 1 | G-2 metric sink | ~40 lines | — |
| 2 | G-1 step fields | ~60 lines | G-2 (shares the span plumbing) |
| 3 | G-3 failure counters | ~120 lines + one subcommand | — |
| 4 | G-4 advisor persistence | ~80 lines | G-3 (same session-store write) |
| 5 | G-6 enhanced-keys probe | ~150 lines + tests | — |
| 6 | G-5 config drift | ~100 lines | — |

Every row ships its own regression test, mutation-proved RED by reverting the row, and its own
iteration + push. No row is allowed to "mostly work": if the check cannot be run, the row is
not done.
