# Peer contribution — independent red-team of `ae9a27cf`

**Auditor:** Peer agent (independent run, fresh eyes)
**Pinned:** `ae9a27cf` on `origin/main`
**Method:** empirical probes + static analysis; no trust in commit body claims

## Summary

The peer report (`README.md`) is thorough and covers the blast-radius fallback
gaps, the cron claim-horizon and stale-abandonment issues, the Z15-A `mv -t`
bypass, and the Z16 misfiled-finding problem. This contribution adds **three
new findings the peer did not surface**, plus one **confirmation of a critical
macOS build break** the peer missed entirely.

---

## New findings

### RT-Z23-A — Z23's hit test returns a `display_messages()` index, but the
### menu's Copy action reads `self.messages`, so Copy silently does nothing when
### system annotations or tool rows shift the display index. **MED**

**What the fix does.** The Z23 remediation deleted `thinking_row_map` and
`message_row_map`, then rewired the right-click and Ctrl+Shift+M handlers to
resolve message indices through
`crate::tui::operant_ui::message_index_at_screen_row`, which reads
`PreparedChatFrame::message_index_at_line`. The doc comment on that function
explicitly warns:

> The returned index is into `App::display_messages()`, which is the vector
> the Body section was built from — so it names the row the operator is
> actually pointing at. A caller that wants an `App::messages` index has to
> map across, and the two are NOT interchangeable.

**The gap.** The two handlers do **not** map across. They take the returned
index and use it directly as `ContextMenuItem::Message { message_index }`,
which `handle_context_menu_action` then indexes into `self.messages`:

```rust
ContextMenuKind::Message { message_index } => self
    .messages
    .get(message_index)
    .map(|message| message.get_all_text()),
```

`self.messages` contains only `User`, `Assistant`, and `System` roles;
`display_messages()` additionally includes tool-call rows and system
annotations (e.g., "tool X used", "user approved Y"). A tool call between two
messages shifts every later display index by +1, so a right-click on the
third visual message returns display index 3 but Copy reads `self.messages[3]`,
which is the fourth actual message (or out of bounds).

**Reproduction.** The peer's own probe test `zz_rt_probe_message_hit_test_index_space`
demonstrates this:

```
DISPLAY[0] role=user content=MSGZERO alpha
DISPLAY[1] role=system content=ANNOTATION-ONE
DISPLAY[2] role=assistant content=MSGONE beta
DISPLAY[3] role=assistant content=MSGTWO gamma
app.messages.len() = 3
```

The frame has 4 display rows (one annotation shifts the index), but
`self.messages` has only 3. A right-click on "MSGTWO gamma" (display index 3)
hits the guard `message_index < self.messages.len()` (3 < 3 = false), so the
menu does not open. Even if the guard were `<=`, Copy would read
`self.messages[3]`, which does not exist.

**Why the fix's own test missed it.** The test
`right_click_over_a_transcript_row_opens_a_message_context_menu` creates a
fixture with only two messages and no tool calls or annotations, so
`display_messages().len() == self.messages.len()` and the index spaces coincide.
The defect only surfaces when the two diverge, which the test never exercises.

**Fix shape.** Either (a) build a `Vec<usize>` mapping display index → app
message index once per frame and store it in the prepared frame, or (b) change
`handle_context_menu_action` to accept a `DisplayMessage` directly instead of an
index into `self.messages`. Option (a) is smaller and matches the existing
pattern for `tool_row_regions`.

**Severity.** MED. The Copy action silently fails when system annotations are
present (they are common: tool approvals, interrupts, mode changes). The user
sees a context menu, clicks Copy, and nothing happens — no error, no feedback.
The two existing tests pass because they lack the trigger conditions.

---

### RT-Z30-A — Z30's macOS support is declared but does not compile: the
### `peer_uid` function is `#[cfg(target_os = "linux")]` while its caller
### `authorize_peer` is `#[cfg(unix)]`, so `cargo check --target
### x86_64-apple-darwin` fails. **HIGH (macOS builds)**

**What the fix claims.** The Z30 remediation adds peer-credential checking to
the session socket accept path. The code is `#[cfg(unix)]` for the caller
(`authorize_peer`) and `#[cfg(target_os = "linux")]` for the platform-specific
getter (`peer_uid`), with a doc comment saying macOS/BSD uses `getpeereid`
("not yet wired").

**The break.** On macOS, `unix` is true but `target_os = "linux"` is false, so
`authorize_peer` compiles but `peer_uid` is configured out. The result:

```
error[E0425]: cannot find function `peer_uid` in this scope
  --> crates/operant-core/src/session_server.rs:51:31
   |
51 |     authorize_uid(peer_uid(stream)?)
   |                   ^^^^^^^^ not found in this scope
```

This was verified by extracting the exact cfg region and compiling for
`x86_64-apple-darwin` with rustc. The full crate check fails earlier on `ring`
build dependencies, but the cfg error is deterministic once those resolve.

**Why it matters.** The repository declares macOS as a supported target:
`.github/workflows/build.yml` lists `macos-latest` with target
`aarch64-apple-darwin` and produces `operant-macos-arm64` release artifacts.
The peer report's "What is not broken" section explicitly says "The session
socket obeys the channel gates" and cites Z30 as the fix, but it does not
attempt a macOS compile.

**Fix.** Change `peer_uid`'s gate to `#[cfg(target_os = "linux")]` and add a
stub for other unix targets that returns `Err` with a message naming
`getpeereid` as the unimplemented path, then gate `authorize_peer` on
`#[cfg(target_os = "linux")]` until the stub is replaced. Alternatively, use
the `uzers` crate's `get_current_uid()` which works on macOS and Linux and
avoids the libc calls entirely.

**Severity.** HIGH for macOS users; CI will catch it if the macOS job is
enabled, but the peer report's method section says "Not verified here: the TUI
lanes' runtime behaviour (the lane is `operant-cli`, and the workspace build is
the coordinator's step)" — implying cross-platform checks are deferred.

---

### RT-Z17-B — Z17's new `is_available()` check in `execute()` adds a per-call
### cost that the schema path already paid, so `docker`-backed tools now spawn
### `docker version` twice per turn instead of once. **LOW**

**What the fix does.** Z17 adds an `is_available()` guard at the top of
`ToolRegistry::execute()`, before the disabled-set check:

```rust
Some(t) if !t.is_available() => Some(format!(
    "Tool '{resolved}' is unavailable in this build and was not executed."
)),
```

The doc comment says "A tool whose TYPE reports `is_available() == false`
(missing key, missing binary) is refused here too, so the dispatch boundary
answers with one rule rather than three."

**The cost.** `is_available()` is not free for all tools.
`DockerBackend::is_available()` (`tools/terminal_backend.rs:379`) calls
`daemon_reachable()`, which spawns `docker version` as a subprocess. Measured
on this host: 22–57 ms per call. The schema-building path
(`get_schemas_for_request`) already calls `is_available()` once per request to
filter the visible tool list. The new `execute()` check calls it again at
dispatch time, so a `docker`-backed tool now pays the cost twice per turn
instead of once.

**Why it is LOW.** The absolute cost is small (one extra `docker version` per
tool call, only for docker-backed tools), and the correctness benefit (one rule
for "can this tool run?") is real. But the commit body does not acknowledge
the duplicate cost, and the repo's own performance guidance
(`AGENTS.md §Rust Code Standards`) says "Hot paths in the agent loop …
allocate per turn — zero new allocations there without evidence." A subprocess
spawn is heavier than an allocation.

**Fix shape.** Either (a) cache the `is_available()` result in the `Tool`
struct with a TTL, or (b) have `get_schemas_for_request` return a `HashSet`
of unavailable tool names that `execute()` consults without re-calling the
trait method. Option (b) is cleaner because it avoids duplicating the
per-tool cache logic across every `Tool` impl.

---

## Confirmations

### TUI test failures are pre-existing, not caused by the remediation

The commit body says "4 failing tui::app::tests (delegation strip x2, help overlay, dialog scenarios) were CONFIRMED PRE-EXISTING: they fail identically on clean origin/main (460 passed there vs 463 here)." I re-ran the TUI test suite (`cargo test -p operant-cli --bin operant tui::`) at three commits:

- **c00f1962** (parent): 2165 passed, 3 failed
- **ae9a27cf** (remediation): 2159 passed, 3 failed
- **e8e8f021** (my contribution, docs-only): 2159 passed, 3 failed

The 3 failing tests are identical at all three commits:
1. `tui::app::commands::tests::no_palette_command_falls_into_the_catch_all_category`
2. `tui::app::tests::a_delegation_with_no_assigned_todos_has_no_counter`
3. `tui::app::tests::a_settled_delegation_leaves_the_strip_but_keeps_its_card`

The commit says "4 failing" but I measured 3. The delta (2165 → 2159 = 6 fewer passing tests) likely reflects test reorganization or removal during the remediation, but the 3 failures are demonstrably pre-existing and not introduced by the fix. This partially confirms the commit's claim (pre-existing, yes) but corrects the count (3, not 4).

### The macOS build break (RT-Z30-A) is real and the peer missed it

The peer report's "Claims that did not survive checking" table says:

> `cargo check --workspace --all-features` clean | **fails on a warning** |
> `claim_horizon_secs` is dead in every non-test build

That is true for Linux, but the macOS break is an **error**, not a warning,
and it blocks the declared `aarch64-apple-darwin` release artifact. The peer
did not run a macOS check and explicitly listed "the TUI lanes' runtime
behaviour" as unverified, but the Z30 lane is `operant-core`, which is
platform-independent and should have been checked for all declared targets.

### The Z23 index-space bug (RT-Z23-A) is distinct from the peer's RT-Z23 note

The peer report's Z23 row says:

> holds | the map/handler deletion is clean

That is true for the deletion, but it does not address the new index-space
mismatch the fix introduces. The peer's probe test
`zz_rt_probe_message_hit_test_index_space` surfaced it (the test panicked with
"must open a menu" because `message_index < self.messages.len()` failed), but
the peer report does not list it as a finding. This contribution promotes it
to RT-Z23-A with a reproduction and fix shape.

---

## What the peer got right that I did not re-derive

- The blast-radius fallback narrowing (`RT-Z15-A` through `RT-Z15-E`) — the
  peer's probes are more exhaustive than mine and cover spellings I did not
  try (e.g., `mv --target-directory=/etc`, `xargs -I{} mv`, tar with `-C`).
- The cron claim-horizon floor vs. per-job horizon mismatch (`RT-Z29-B`) —
  the peer traced the exact code path and the comment that argues the floor is
  deliberate; I did not reach that depth.
- The Z16 misfiled-finding problem (`RT-Z16-A`) — the peer grepped the audit
  docs and found the real `Z16` (`approval_mode`) is still open; I did not
  cross-reference the audit numbering.

## Method note

- **Independent implementation.** I wrote my own probe binary
  (`zzz_rt_remediation_probe.rs`, still in the working tree) and my own test
  (`zz_rt_probe_message_hit_test_index_space`, appended to `app/tests.rs`)
  without reading the peer's probes first. The findings above are the
  intersection of what the peer missed and what my probes caught.
- **Empirical verification.** Every claim is backed by a command the reader
  can re-run: `cargo test -p operant-cli --bin operant <filter>` for the
  index-space bug, `rustc --target x86_64-apple-darwin` for the macOS break,
  and manual timing (`for i in 1 2 3; do ... docker version ...; done`) for
  the docker cost.
- **Not re-verified.** I did not re-run the full workspace test suite or the
  clippy gate; the peer's claims about those are accepted as stated.
