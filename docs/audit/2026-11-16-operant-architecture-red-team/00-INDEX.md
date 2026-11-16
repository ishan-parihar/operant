# 00-INDEX — operant architecture red-team + ponytail audit: synthesis

**Date:** 2026-11-16
**Pinned revision:** `897a8baf` (`feat(iter-2477): a written description
must fit the row the index shows`).

**Pinned three times.** The audit began at `25b86ae5` (iter-2472); peers landed
iter-2473 (bar retraction), iter-2474/2475 (project-skill quarantine, doctor
credential check) and iter-2476/2477 while lanes were running. Every finding
below was re-verified at `897a8baf`. **One finding did not survive: `Z34` is
FIXED by iter-2474** and is reported as closed rather than as an open HIGH.
The ranked list has been re-counted accordingly.
**Method:** `/ponytail-audit` + `/rust-best-practices`, ten parallel lanes
(A–J) each writing its own evidence doc, plus a coordinator verification lane
(Z) that re-derived every load-bearing claim before it was promoted here.

## How to read this file

- **Nothing in the ranked list below is a lane's assertion taken on faith.**
  Each entry names the coordinator doc that re-derived it, with `file:line`
  from the pinned revision. Where a lane asserted and the coordinator could not
  confirm, the entry is marked **[ASSERTED]** and excluded from the ranking.
- The ten lane docs are the evidence base and are worth reading in full:
  `A-agent-loop-and-dispatch.md`, `B-tools-and-guardrails.md`,
  `C-config-and-secrets.md`, `D-memory-context-persistence.md`,
  `E-gateway-channels-cron.md`, `F-mcp-web-plugins.md`,
  `G-skills-org-curator.md`, `H-tui.md`, `I-cli-surface.md`,
  `J-rust-discipline.md`, `Z-coordinator-verification.md`.
- Severity is **operational impact**, not effort. A HIGH is a defect that
  changes what the software does to the operator's machine, data, or safety
  boundary today.

**Contents:** [The two numbers that frame everything](#the-two-numbers-that-frame-everything) · [Ranked findings — HIGH](#ranked-findings--high) · [Ranked findings — MEDIUM](#ranked-findings--medium) · [The ponytail cut list](#the-ponytail-cut-list--what-to-delete-not-rename) · [What is not broken](#what-is-not-broken-do-not-re-litigate-these) · [AGENTS.md drift](#agentsmd-drift--the-doc-is-describing-a-different-repo) · [Suggested order of work](#suggested-order-of-work) · [Provenance](#provenance)

## The two numbers that frame everything

| measurement | value | where |
|---|---|---|
| coordinator-verified findings | **37** (`Z1`–`Z37`) | this doc |
| of those, HIGH | **10 found, 9 open** (Z34 closed by iter-2474) | ranked below |
| clippy gate status | **RED, exit 1, 52 new warnings** | `Z1`, reproduced by lane J (`Z26`) |
| untracked-and-unignored scratch | **1.3 GB** | `Z9` |

The gate is the headline because it is the one finding that invalidates the
repo's own stated verification contract. AGENTS.md requires
`bash scripts/clippy-warning-gate.sh` to pass before a push, and it has not
passed for 1,509 commits.

---

## RANKED FINDINGS — HIGH

### 1. The clippy warning gate is red, and has been for 1,509 commits
`Z1` · lane J independently reproduced (`Z26`)

`bash scripts/clippy-warning-gate.sh` exits 1 with `seen=52 allowlist=0 new=52
stale=0`. `.ci/clippy-allowlist.txt` is a tracked **0-byte file**, emptied by
`c2a645d3` (iter-1171, 2026-10-22).

AGENTS.md's own iteration loop requires this gate to pass before a push
("If any command fails, the iteration is **not complete**"). Two sessions
independently measured the same 52 warnings, so the contract has been
unmeetable for every iteration since iter-1171 — including the ones that
declare the workspace verified.

**Fix:** refill the allowlist from a real run (`--update`), and fix the 38
production warnings. **Do not** weaken the gate.

### 2. The gate is nondeterministic, so "it passed" is not reproducible
`Z2` · lane J (`Z26`)

Five runs on one unchanged warm tree produced `seen = 31, 52, 31, 53, 52`.
Cause: the script runs two cargo passes against one `target/` —
`--lib --bins` (28 clippy keys) then `--all-targets` (56 keys) — and each
pass's cached results leak into the other. Each pass is deterministic alone;
the union is not.

This is why finding 1 must not be "fixed" by pasting in one run's output:
the number pasted would be true for one tree state and false for the next.

**Fix:** give each pass its own `CARGO_TARGET_DIR` (or `--no-deps`). Ship this
**with** finding 1, not after it.

### 3. `mv` / `cp` / `rsync` / `install` / `scp` are invisible to the blast-radius gate
`Z15` · lane B's `B1` (HIGH), verified with full mechanism

`blast_radius::verb_kind` (`crates/operant-core/src/blast_radius.rs:199`–`:217`)
is a closed `match` with `_ => return None`. Those five verbs are **absent**.
An unrecognized verb gives `verb: None`, which `rate_segment` maps to
`BlastRadius::None` (`:546`–`:551`) — the floor tier — and `screen()`
(`:1199`–`:1203`) returns `None` immediately, **before** the deny-list check at
`:1204`–`:1210`.

So `mv /etc /tmp/x`, `cp ~/.ssh/id_rsa /tmp/x`, `rsync -a /etc/ /tmp/x` and
`install -m 644 /etc/shadow /tmp/x` all miss layer 1 (hardline), miss layer 2
(patterns) and score `None` at layer 3 — the answer is **allowed**. Through a
modelled verb the same targets are denied: `rm -rf /etc` draws both an L1 hit
and `DeniedPath{system directory}`.

The deny list is the strongest control in this seam and it is reachable through
a seven-verb table.

**Fix:** add the file-moving/copying family to the table, each with a test that
a scoped in-repo copy still rates `Scoped`.

### 4. A cron job can fire twice after a crash, repeating its side effects
`Z29` · lane E's `E7`

`cronjobs/scheduler.rs:358`–`:410`: the agent/script runs **first**, and
`mark_job_run` — the only write that advances `next_run_at` — runs after.
`tick()` selects due rows with no in-progress marker (`db.rs:1023`–`:1031`).

Crash between dispatch and `mark_job_run` and `next_run_at` still reads the
old time, so the next `tick()` after restart fires the job again. No
idempotency key, no run-history check before dispatch. An agent job that sends
a message or posts to a channel does it twice, and the transcript does not
distinguish the runs.

**Fix:** a claim row written **before** dispatch, not a `next_run` write after
it.

### 5. `/model roles` opens a panel that paints nothing while owning the keyboard
`Z22` · lane H's `H1`

`crates/operant-cli/src/tui/models_panel/` is one 391-line `mod.rs` with state,
a key handler and an opener — and **no renderer**:
`rg 'models_panel' crates/operant-cli/src/tui/render/ crates/operant-cli/src/tui/operant_ui/`
exits 1.

`/model roles` → `app/commands.rs:772`–`:775` sets `models_panel.visible`, the
frame paints the unchanged chat, and `key_handling.rs:1247`–`:1250` routes
**every** key into the invisible panel and returns `false`. Arrow keys move an
invisible cursor; `x` on an assigned slot calls `persist_config_key(key_path,
"")` — **writing the config from a panel the operator cannot see**. Escape is
the only way back.

Aggravating: `models_panel.visible` is **not** in `overlay_flags()` (29 names,
`app/commands.rs:93`–`:133`), so `any_modal_open()` reports false while the
panel owns the keyboard. `models_panel/mod.rs:73`/`:86` carry
`#[cfg_attr(not(test), allow(dead_code, reason = "the render half is the next unit"))]`
— the render half was announced and never landed.

The sibling `subagent_navigator` / `agent_picker` do have a paint path
(`render/operant_overlays.rs:538`–`:547`), so the pattern exists.

**Fix:** add the overlay row plus a `paint_models_panel`, or delete the panel
and repoint `/model roles` at the inline picker's Roles scope. Do **not** merely
add the flag to `overlay_flags()` — that hides the defect one layer down.

### 6. Two mouse hit-test maps are never written: right-click and
### click-to-collapse-thinking are dead
`Z23` · lane H's `H5`

`App.thinking_row_map` and `App.message_row_map` (`app/mod.rs:995`/`:997`) are
initialised empty (`app/init.rs:415`–`:416`) and **nothing inserts into them in
production** — the only `insert` in the tree is a test (`app/tests.rs:4650`).

Their readers therefore always take the `else` branch: right-click over the
transcript always calls `dismiss_context_menu()` (`mouse.rs:1244`), Ctrl+Shift+M
falls back to a selection-only menu, and the click-to-collapse-thinking handler
(`mouse.rs:1331`) is unreachable — which also means `thinking_expanded` can
never be non-empty, since its only writer is that dead branch (`mouse.rs:1335`).

The live paths use mechanisms that do get a frame-time writer: the pinned-todo
band publishes a rect via `pinned_todo_more_area()` (`ui_viewport.rs:1688`) and
tool rows via `TOOL_ROW_REGIONS` (`:1697`). The comment at `:1686` still cites
`thinking_row_map` as the working precedent, which is what sends a reader to
the one map nothing writes.

**Fix:** publish the maps during render (same shape as `TOOL_ROW_REGIONS`), or
delete them and the two dead handlers.

### 7. The content-integration install path verifies nothing and does not warn
`Z24` · lane F's `F3`

`crates/operant-core/src/binary_updater.rs:294`–`:306` is the whole content
path: download, `install`, return. `verify_sha256` has exactly two callers
(`:316`, `:574`) and **neither is on this path**.

Forty lines below, the binary path explicitly branches on `release_has_sums`
and, when no manifest exists, emits a `tracing::warn!` whose comment reads:
*"But it must not be SILENT — 'a verification that silently never runs reads as
protection' is exactly the defect class this row is about."* That reasoning is
correct and applies identically to the content tarball, which has no such
branch.

A compromised release, a CA-trusting proxy, or a compromised `gh` host can ship
`skills/ponytail/SKILL.md` with arbitrary content. If the guard verdict is
`Caution` it installs with a log warning — and becomes the **frozen prompt
prefix** on every subsequent model request (`ponytail.rs:155`–`:181`).

HIGH rather than BLOCKER: it needs control of the release or a MITM position,
the fetch is TLS, and ponytail's posture defaults OFF. It is HIGH because this
is a **self-updating content integration whose purpose is to install silently
in the background**, and it is the one path with no verification and no
operator-visible signal.

Note for the fix: ponytail releases carry no assets by design
(`ponytail.rs:33`), so wiring `release_has_sums` would always no-op. The fix is
to publish a sums file **or** state the trust model explicitly.

### 8. A project root's skills load as `Integration` and are never scanned;
### `SkillOrigin::Project` is a dead variant — **CLOSED by iter-2474 (`baba858e`), verified fixed**
`Z34` · lane G's `F-2` (lane G left it PENDING; re-derived here, then closed)

**Status: FIXED at `897a8baf`.** A peer's iter-2474 landed while this audit was
running and closed the finding exactly as the fix direction below prescribed.
Verified at the tip: `skills.rs:695` now calls
`self.load_root(&project, SkillOrigin::Project, usize::MAX, &mut loaded)` —
a third `load_root` site passing the previously-dead origin — and `:767`
gates it with `is_quarantined_skill(&path)`, a fail-closed check that
quarantines a `dangerous` verdict, an uncanonicalizable directory, and a
scanner panic alike. `RootIssue` gained a `"quarantined: dangerous scan
verdict"` arm (`:198`) so `skills list` surfaces it. iter-2473 added the
content-digest scan cache that makes the check affordable on the load path.

The finding is kept below, marked, because it is the audit's clearest example
of a lane finding being **right and then irrelevant** — and because the
original evidence is what makes the fix verifiable.

`SkillOrigin` declares `Project` with a documented behaviour
(`skills.rs:41`–`:45`: "readable and invocable; `create`/`delete` refuse it").
`SkillRoots::resolve` does build a project root (`:282`–`:289`). But
`SkillManager::from_roots` (`:571`–`:581`) collapses the root list into
"primary + everything else", and `load_all` (`:620`–`:632`) passes only two
origins: `OperatorPool` for the first root, `Integration` for every other.

`load_root` is called from exactly those two sites.
**`SkillOrigin::Project` is never passed to `load_root` anywhere.** Its only
production mention is a comparison at `:959` in a `create`/`delete` refusal that
can never be true.

Consequences:
1. A project skill is labelled `Integration`, so every downstream rule that
   distinguishes them (the `[integration]` marker, pool-wins-collision, the
   "an integration skill can never be created or written in place" refusal)
   applies to the repository's own skills as if they were vendor content.
2. **No scan.** `scan_skill` has exactly two production callers, both in
   `cmd_skills.rs` (`:751`, `:868`) — the explicit install/check commands. The
   loader never imports `skills_guard` at all. Skills found in a **cloned
   repository's `skills/`** are walked, parsed and made invocable with no guard
   reading a byte.

Failing case: clone a repo carrying `skills/do-us-a-favour/SKILL.md`; it loads
as an `Integration`-labelled skill, appears in `/skills`, is invocable, and
nothing scanned it. The only signal is `operant skills check`, which the
operator must know to run.

The repo has the mechanism — it applies `scan_content_against_guard` to vendor
tarballs (`binary_updater.rs:746`–`:792`) while skipping it for a stranger's
git checkout.

### 9. The session socket has no peer authentication: any same-UID process is
### the operator
`Z30` · lane E's `E5`

*(Numbering is the order findings were verified, not a re-sort; the closed
item 8 keeps its slot. Open HIGHs are 1–7 and 9–10.)*

`session_server.rs:119` discards the peer address:
`let (stream, _) = match listener.accept().await {` — and there is no
`SO_PEERCRED` / `getpeereid` / uid check anywhere.

The only protection is filesystem: 0600 tightened at `:108`–`:115` (warns
rather than fails), plus a 0700 home. Any process running as the same UID —
a language server, a build script, a browser helper, another agent's tool — can
`connect()` and send `WireMessage::Prompt`, which is an **unsandboxed agent turn
with the full toolset**.

The code's own comment (`:104`–`:107`) states the stakes: "the socket carries
full control of the agent and its tools" — then grants that control on UID
alone. Its named mitigation (0700 home) is bypassed by any `--socket` path
outside the home.

**Severity:** HIGH on the multi-agent fleet this repo documents (23 live
worktrees, peer agents in one tree), where same-UID is not a trust boundary;
MEDIUM on a single-user workstation with nothing untrusted running as the
operator.

### 10. `ReasoningParamRejected` is handled only in the streaming arm, so the
### compression path re-sends the rejected parameter
`Z16` · lane A's `A5`

The streaming arm calls `mark_effort_unsupported` on
`ReasoningParamRejected`. The **non-streaming compression arm never does**, so
`fitted_request` keeps sending the `reasoning_effort` the provider just
rejected.

Failing case: a provider that rejects reasoning parameters and a session that
overflows context — every compressed retry re-sends the rejected parameter
instead of degrading once and remembering.

**Fix:** call `mark_effort_unsupported` from the compression arm too, or hoist
the handling to where both arms converge.

---

## RANKED FINDINGS — MEDIUM

| # | finding | ref |
|---|---|---|
| 11 | Gate's test exemption misses in-file `#[cfg(test)]` modules in `src/` — 14 of 52 warnings are test-only noise | `Z3` |
| 12 | `ToolRegistry::execute()` does not enforce the disabled set, so `operant tools disable` does not bind in the loop | `Z17` |
| 13 | `ThinkingSignature` shadows `ReasoningParamRejected` with a vacuous test, so a real provider rejection is misclassified | `Z14` |
| 14 | An empty `SKILL.md` passes every content-install gate, yielding a silently empty ruleset | `Z25` |
| 15 | `operant cron run <id>` bypasses the daemon lock, so a manual run races a scheduled run of the same job | `Z31` |
| 16 | Blocking filesystem + JSON on the webhook request path — a remote party pacing your worker threads | `Z33` |
| 17 | No retention on `messages` / `context_ledger` / `session_events` / `verification_events`; `context_ledger` appends a 16-column row per request | `Z37` |
| 18 | The generic webhook HMAC path has no replay window, while Slack and Stripe both enforce 300s | `Z21` |
| 19 | `crates/operant-cli/src/tui/adapter_types/output_styles.rs` (18 LOC) is fully dead; its only importer imports without using | `Z4` |
| 20 | `should_fallback` is constructed 76 times and read by **nothing**; the code says so explicitly at `fallback.rs:86`–`:89` | `Z14` |
| 21 | `~2,400` of `operant-config`'s 6,725 LOC is dead — `policy.rs` `SecurityPolicy`/`command_risk_level` duplicate a live classifier | `Z5` |

---

## THE PONYTAIL CUT LIST — what to DELETE, not rename

Ordered by LOC removed per unit of risk. Every entry is dead by grep, not by
judgment.

| # | delete | LOC | evidence |
|---|---|---|---|
| 1 | `crates/operant-config` dead modules: `policy.rs`'s `SecurityPolicy` + `command_risk_level`, `secrets.rs`, `provider_aliases.rs`, `traits.rs`, `autonomy.rs` | ~2,400 | `Z5` — one external consumer of the whole crate (`split_unquoted_segments`, `tools/file_tools.rs:182`) |
| 2 | `historian.rs` + the `compartments` table, its 3 indexes and its 3 methods | 720 + DDL | `Z12`, `Z36` — writer deleted iter-2404; that note says "ZERO callers" |
| 3 | `compression_locks` table + index + 3 methods + 3 tests | DDL + 6 fns | `Z12`, `Z36` — zero callers outside `database.rs` |
| 4 | `adapter_types/output_styles.rs` | 18 | `Z4` — only importer imports without using |
| 5 | `codex_reasoning_items` / `codex_message_items` / `platform_message_id` columns | 3 cols | `Z13`, `Z36` — set to `None` at every site outside `database.rs` |
| 6 | `unsafe impl Send/Sync for InterruptFlag` (both) | 2 lines | `Z27` — `Arc<AtomicBool>` is auto-`Send`+`Sync`; the impls assert what the compiler already guarantees |
| 7 | `models_panel/` OR its renderer (pick one — do not leave it half-built) | 391 | `Z22` |
| 8 | `thinking_row_map` + `message_row_map` and the two dead handlers | ~40 | `Z23` |
| 9 | `PlatformAdapter::handle_update` — dead for all seven adapters | 5 impls | `Z32` |

**Total: roughly 3,200 LOC and 4 DB objects, with no behavior change.**

Cut 1 is the big one and the one to sequence carefully: `policy.rs` is 3,791
LOC with exactly one live consumer. Deleting the dead half is mechanical once
that consumer is repointed.

---

## WHAT IS NOT BROKEN (do not re-litigate these)

Recorded because a red-team's output is misread as a defect list. Each of these
was checked and **holds**:

- **Eviction tool-pairing invariant** — a tool result is never orphaned from
  its call. Structural, holds.
- **Fan-out preserves partials** — a cancelled fan-out does not lose completed
  siblings.
- **Decay curve clamps** — `H = H50·2^((I-50)/D)/max(p,0.10)` cannot divide by
  zero and cannot understate the reserve.
- **Memory is memory-wire-only** — the legacy providers are genuinely gone;
  unknown names downgrade to builtin, as documented.
- **Gateway send/receive claims are 6/6 CONFIRMED, 0 refuted** (lane E):
  Slack/WhatsApp/Email/SMS are outbound-only, Webhook is inbound-only. AGENTS.md
  is accurate here.
- **HMAC verification is constant-time** (both named variants).
- **`workspace.lints` is usable** in this environment — the AGENTS.md note
  claiming manifest-level `[workspace.lints]` inheritance is unusable is
  **wrong** (lane J). Six crates already inherit it. One crate,
  `operant-os`, declares no `[lints]` section.
- **`unsafe` is rare** — 37 production blocks across 7 crates, 25 with an
  explicit SAFETY comment.
- **24 error classes**, one skill-root resolver, dashboard `Host` guard
  unconditional.
- **The session socket obeys the channel gates** — ESTOP,
  `max_concurrent_sessions`, slot binding; and a channel-owned approval is
  genuinely refused on the socket (lane E's `E3`/`E4`, enforcement verified).

---

## AGENTS.md DRIFT — the doc is describing a different repo

| claim in AGENTS.md | reality | ref |
|---|---|---|
| "CLI (52 command variants)" | enum has **53**; `continue` is missing from the list | `Z35` |
| "release line `0.2.1`" | the tree is at **0.3.0** | `Z7` |
| "`App::overlay_flags()` names **30** of them" | the array has **29** names | `Z6`, `Z10` |
| "manifest-level `[workspace.lints]` inheritance is unusable in this environment" | **false** — 6 crates inherit it | `Z26` (lane J) |
| "iteration-label authority is origin/main" | correct, and the reason is sound | — |

The CLI count is the interesting one: that section's own closing paragraph
warns *"Re-derive it rather than trusting it"*, and the list has drifted by the
same off-by-one it was corrected for. The lesson generalizes — the paragraph
should be a script, not a sentence.

---

## SUGGESTED ORDER OF WORK

1. **Fix the gate first** (`Z1` + `Z2` together). Everything else is verified
   against a baseline the repo cannot currently reproduce. Refilling the
   allowlist without fixing the determinism just moves the goalposts.
2. **Then the two HIGH correctness bugs that cost data or config**:
   cron double-fire (`Z29`), `/model roles` writing config invisibly (`Z22`).
3. **Then the three HIGH safety/seam bugs**: blast-radius verbs (`Z15`),
   content-install verification (`Z24`), project-root scanning (`Z34`).
4. **Then MEDIUM**: the session-socket auth decision (`Z30` — needs an
   operator call on the trust model), disabled-set enforcement (`Z17`),
   the compression-arm `ReasoningParamRejected` fix (`Z16`).
5. **Then the ponytail cuts**, largest first (cut 1 → cut 4 → the rest), each as
   its own iteration with the compile gate green.
6. **Then the doc drift**, in one docs-only iteration.

## Provenance

- Lane docs: 10 files, 2,152 lines of evidence, in this directory.
- Coordinator verification: `Z-coordinator-verification.md`, 1,197 lines,
  `Z1`–`Z37`, all VERIFIED. Two earlier findings were reclassified from
  ASSERTED after the coordinator traced them (`Z14`, `Z21`).
- **Re-pinned twice, mid-audit.** The audit began at `25b86ae5` (iter-2472);
  peers landed iter-2473 through iter-2477 while lanes were running. Every
  finding was re-verified at `897a8baf` and 36 of 37 still hold. The one that
  closed is `Z34`, fixed by iter-2474 — see item 8 above. The three TUI
  findings the bar-retraction diff could plausibly have invalidated were
  re-checked specifically and survive: `overlay_flags()` is still 29 names
  (AGENTS.md still says 30), `models_panel` still has no renderer, and the two
  mouse maps are still unwritten outside `app/tests.rs:4650`.
- Two lanes' findings were confirmed by a **second** independent session:
  lane J reproduced `Z1`/`Z2`/`Z3` exactly, and lane D reproduced `Z12`/`Z13`
  with the same `file:line` evidence. That cross-check is why those five are
  treated as settled rather than as hypotheses.
- Lane C (config/secrets) and lane I (CLI surface) did not complete; their
  scope was covered by coordinator findings `Z5` and `Z35` respectively.
- Lane G's `F-1` (shadowed skill silently dropped) was **re-derived as FIXED**
  by iter-2468 (`skills.rs:737`–`:762` now reports every shadow) — the prior
  pass's HIGH is closed and must not be re-opened.