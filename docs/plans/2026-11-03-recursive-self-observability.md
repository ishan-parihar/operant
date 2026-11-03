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

3. **"or page refreshes" — the second half of the report does NOT reproduce.** Checked with the
   documented oracle for this exact defect class rather than trusted: `--frame-per-event`
   captured every painted frame across a full submit-and-reply turn (60 frames), and
   **no frame ever carried the prompt twice**. The composer's text clears on submit, the
   transcript carries the user row exactly once, and `scroll_emit` emits only settled
   transcript rows and never the composer (`scroll_emit.rs` has no reference to
   `prompt_input`). `note_resize` touches scroll anchoring only. So whatever the operator saw
   on a refresh is not reproducible headlessly — most likely the same failed-turn duplicate
   from item 1, which is now fixed, or a terminal-side artefact. Recorded rather than guessed
   at: if it recurs on the live path, the capture command to run is
   `operant tui debug simulate --keys '<the prompt><enter>' --frame-per-event
   --capture-dir <dir>` and the check is
   `grep -l '<prompt>' <dir>/frame-*.txt | wc -l` against the per-frame occurrence count.

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
| `terminal_probe.rs` | queries the terminal's actual kitty flags and reads the replies back | push only, no query — `keyboard_enhancement.rs` | **G-6: real gap (the query half only)** |
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

**CORRECTED 2026-11-03: the app-side half is done and tested, but the comparison
to codex stands and is bigger than I first thought.** Two separate claims, kept
apart because I conflated them and got the first one wrong:

**Part 1 -- operant's own push exists and works (DONE, NOT a gap).**
`keyboard_enhancement.rs` pushes `DISAMBIGUATE_ESCAPE_CODES |
REPORT_EVENT_TYPES | REPORT_ALTERNATE_KEYS` via crossterm's
`PushKeyboardEnhancementFlags` at `tui_app.rs:127`, ownership-gated on an
`AtomicBool`, balanced by the pop at teardown and in the panic hook. Three tests
pass, including `push_and_pop_write_the_kitty_sequences`, which pins the exact
bytes (`CSI >7u` push, `CSI <1u` pop) so a crossterm upgrade that changes the
encoding fails loudly. And
[`scripts/shift-enter-live.sh`](../../scripts/shift-enter-live.sh) drives the
REAL `operant chat` on a REAL pty and writes the genuine kitty encoding
`CSI 13 ; 2 u` at it: PASS on the real bytes, FAIL on a mutant where that byte is
swapped for a bare `\r`. That closes the decode question -- the bytes a
compliant terminal emits once the push is honoured really do become Enter+SHIFT
here.

**Part 2 -- operant cannot know whether the terminal AGREED (STILL A GAP).**
This is the part I dismissed too fast, and codex proves it is real. codex's
`codex-rs/tui/src/terminal_probe.rs` (968 lines) sends `CSI ?u` and
`CSI c` / `CSI 6n` / `OSC 10;?` / `OSC 11;?` on startup with a bounded deadline,
READS the replies back, parses the keyboard flags with
`find_keyboard_flags`, and derives four states (`Supported`,
`SupportedAndFallback`, `UnsupportedFallback`, `Pending`).
`tui.rs:517` runs it as `StartupKeyboardEnhancementProbe::Query` by default --
`Skip` only when `keyboard_modes::keyboard_enhancement_disabled()`. It gates its
footer on the answer (`footer_insert_newline_key`,
`chat_composer.rs:4431`), so Shift+Enter is only advertised when the probe
actually succeeded. It also replays bytes consumed during the probe back through
crossterm (`buffer_input`) so no keystroke is swallowed, and keeps
`MAX_TERMINAL_CSI_BYTES = 64` so a split reply cannot deadlock the startup.

Operant pushes and hopes. If the terminal is a multiplexer stripping the
sequence, or an old one, or an SSH client that eats it, operant still advertises
Shift+Enter and the operator gets the original bug back with no diagnostic.

**Build (the honest, smaller version of what I originally proposed):** mirror
codex's query at startup -- write `CSI ?u`, read the reply under a bounded
deadline on a duplicated tty with a nonblocking read, parse the flags, store the
answer. Do **not** build codex's DA1/OSC/DEFAULT-colors/SSH half; the keyboard
bit is the only one that answers a defect an operator actually reported. Then
make the composer hint row and `/terminal-setup` tell the truth: when the probe
says unsupported, say so and name Ctrl+J / Alt+Enter (which already work) rather
than advertising Shift+Enter.

**Check:** the hint row differs for injected `Supported` / `Unsupported`
replies, driven by a byte stream in a test -- the same shape codex's own probe
tests use. Plus the existing `push_and_pop_write_the_kitty_sequences` keeps
pinning the push bytes.

**Two traps to avoid, learned from codex:** replay the bytes consumed during the
probe back through crossterm (`buffer_input`) or the operator's early keystrokes
disappear; and cap the reply scan (`MAX_TERMINAL_CSI_BYTES`) so a terminal that
answers only part of the query cannot hang startup forever.

**BLOCKER, verified 2026-11-03 — do not plan this as a copy of codex's code.**
codex reaches for `crossterm::event::buffer_input`, and that function does not
exist in the crossterm we depend on. `codex-rs/Cargo.lock` pins
`git+https://github.com/openai-oss-forks/crossterm?rev=ed1cdab335221515706178d68495bba2aed1924f`
(`codex-rs/Cargo.toml:673`) — a **fork** that added it. We use upstream
`crossterm = "0.29"`, whose whole event surface is `poll()` and `read()`
(confirmed against the vendored 0.29.0 source: `event.rs` exposes nothing else,
and no feature flag unlocks an inject path). Worse, `UnixInternalEventSource`
(`event/source/unix/tty.rs`) owns the tty fd privately and offers no accessor,
so there is no way to park bytes where crossterm will find them.

The consequence is that codex's *design* ports but its *mechanism* does not. A
correct operant version has to own its own tty: open `/dev/tty` (or dup stdin),
set it nonblocking, read the `CSI ?u` reply directly, and only then hand the
stream to crossterm. Two approaches, both with a real cost, and the choice is an
owner decision rather than a detail:

- **A. Probe on a SECOND fd.** Cheap to write, but the reply bytes may be
  consumed by whichever reader wins the race, so you still need somewhere to
  buffer a byte crossterm never saw — which is exactly the problem upstream
  leaves us no API to solve. This can silently EAT the operator's first
  keystrokes, which is a worse bug than the one it fixes.
- **B. Probe BEFORE crossterm's event loop starts, on the one fd crossterm will
  later use.** Cleaner ownership, no race. Needs an explicit policy for surplus
  bytes typed during the probe (drop / queue / replay into the composer), plus a
  bounded deadline and the capped scan codex uses.
- **C. Infer it at RUNTIME instead of probing at startup -- recommended, and it
  needs no new fd and no fork.** Upstream crossterm makes the two cases
  observably different in the decoded event: `parse.rs:92` maps a bare `\r` to
  an **unmodified** `KeyCode::Enter`, while `:203` routes `CSI ... u` to
  `parse_csi_u_encoded_key_code`, which yields keycode 13 **with** the shift
  modifier (`:413`, `:1244`). So a chord the operator pressed and that arrived
  as plain Enter IS the evidence. Record it once ("this terminal does not
  report enhanced keys") and let the composer hint row and `/terminal-setup`
  tell the truth from then on. No keystrokes can be eaten, because nothing is
  read outside crossterm's own loop, and the only cost is that the first
  Shift+Enter press behaves as today before the switch flips.

  Note what this is NOT: it is not a probe, so it cannot warn a user who never
  presses Shift+Enter. That is acceptable -- the hint row only matters once the
  operator reaches for the key anyway, and the Ctrl+J / Alt+Enter fallbacks are
  advertised from the start regardless.

**Owner decision needed** (recorded in ASKS Q4): build the runtime inference (C,
recommended — no new fd, no fork, and no keystroke can be eaten), or do you want
the codex-style startup probe, in which case its route must be chosen too (B owns
our own fd; A is rejected because it can eat the operator's first keystrokes; a
crossterm fork like codex's is rejected as a supply-chain and maintenance
commitment on a hot input path, for one function).
Until that is settled, this row is not safely buildable — and building it blind
risks losing input, which is far worse than an imprecise hint row.



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
| 3 | G-6 terminal enhanced-keys support — the only remaining half of G-6 (the PUSH is already built, tested, and live-verified; this is the "does the terminal agree" half) | **BLOCKED** on ASKS Q4's route choice — route C (runtime inference, recommended) is ~40 lines and needs no fd or fork; route B (own fd + startup probe) is ~120 and needs a surplus-byte policy | ASKS Q4 |
| 4 | G-3 failure counters | ~120 lines + one subcommand | — |
| 5 | G-4 advisor persistence | ~80 lines | 4 (same session-store write) |
| 6 | G-5 config drift | ~100 lines | — |

Every row ships its own regression test, mutation-proved RED by reverting the row, and its own
iteration + push. No row is allowed to "mostly work": if the check cannot be run, the row is
not done.
