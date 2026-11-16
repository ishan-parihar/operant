# RT-4 — red-team of `ae9a27cf` (iter-2490): the session socket (Z30) and the compression-arm reasoning fix (Z16)

> **STATUS (updated 2026-11-16, commit `d7ecc58f` / iter-2492).** `RT-Z30-A` is
> **CLOSED**: the ACP socket now calls the same `0600` mode tightening and the
> same peer-uid check as the session socket, both moved into a new shared
> `operant_core::socket_auth` module so a third socket server has one obvious
> thing to call. The gate's tests moved with it. `RT-Z30-B` (no peer check off
> Linux, because `getpeereid` is unwired) is **OPEN by decision and recorded**
> rather than papered over: the callers gate on `target_os = "linux"`, so the
> absence is visible at the socket. `RT-Z30-C` (uid-equality is the whole trust
> model, root always connects) and `RT-Z30-D` (the regression test was not
> mutation-shaped) are addressed as far as they can be: the end-to-end test now
> drives the public gate and a foreign uid, and stubbing it fails the suite.
> `RT-Z16-*` are **NOT addressed** by iter-2492 and remain open.


**Audited:** 2026-11-16
**Pinned revision:** `ae9a27cf` — `fix(iter-2490): six-lane remediation of the 2026-11-16 red-team (Z15-Z31)`
**Source findings:** `docs/audit/2026-11-16-operant-architecture-red-team/Z-coordinator-verification.md`
(`Z30` at `:915`), `00-INDEX.md` item 10 (`:280`), lane A's `A5`
(`A-agent-loop-and-dispatch.md:132`)
**Remediation charter:** `docs/audit/2026-11-16-remediation-fleet/CHARTER.md` W5 (`:175`) and W6 (`:199`)
**Method:** read `git show ae9a27cf` per file, re-derive the mechanism, then probe it
with (a) scoped tests, (b) a **mutation test** on each fix, (c) two purpose-built
example probes against the real `run_session_socket` and the real `OperantAgent::run`.
Every row names a command or a file:line.

## Verdict summary

| claim in `ae9a27cf` | holds? | note |
|---|---|---|
| Z30 — peer uid checked from `SO_PEERCRED` | **holds, and it is checked per connection before anything is served** | `session_server.rs:135-143`. But see `RT-Z30-A`/`B`/`C` for the gates it does *not* cover |
| Z30 — 0600 tightening is fatal | **holds** | `session_server.rs:121` returns `Err` before the accept loop; measured 0600 under a 0777 dir |
| Z30 — a foreign uid cannot read `Hello` | **holds on Linux** | the drop precedes `serve_client` |
| Z16 — non-streaming arm now calls the shared helper | **holds** | `run.rs:2325`; mutation-proved (reverting it fails 3 of 5 tests) |
| Z16 — "hoisted into ONE helper both arms call" | **holds for those two arms** | but it is *not* "both arms of the request ladder" — there is a THIRD `chat_streaming` consumer and a fourth `fitted_request` producer. See `RT-Z16-A`/`B` |
| Z30 — regression tests are mutation-shaped | **NO** | `RT-Z30-D`. Removing the gate from the accept loop breaks **zero** tests |
| Z16 — regression tests are mutation-shaped | **yes** | reverting the non-streaming call fails 3 of 5, incl. the "field is absent" assertion |

**Bottom line.** Z30 is a real, correctly layered fix — the uid check is on the
right syscall, in the right place, against the right uid, on every connection,
before any byte is served. Z16 is a real fix for the arm the audit named, and its
test is genuinely mutation-shaped. The remediation's weakness is the same one the
sibling RT-4 rows found: **both fixes were written to the exact spelling the audit
named, and each leaves the neighbouring spelling of the same class open.**
For Z30 the neighbours are the *sibling socket* (ACP, same authority, no gate at
all), the *platform* (`getpeereid` unwired, so macOS silently loses the gate), and
*the test*, which cannot fail. For Z16 the neighbour is the *third streaming
consumer*, which re-sends the rejected field on the drop-replay path — measured.

---

## Z30 — the session socket peer authentication

### RT-Z30-A — the ACP daemon socket has the SAME authority and NEITHER gate. It binds via the shared helper and keeps no mode and no peer check. **HIGH**

`Z30` was scoped to `session_server.rs`, and that file is fixed. But the charter
names the seam by path, and there is a **second UNIX-socket server in this crate
with strictly more authority and strictly less protection**:
`operant_core::acp::server::run_socket_server`
(`crates/operant-core/src/acp/server.rs:45`).

It shares the bind helper — `bind_unless_live` at `:53`, the *same function*
`session_server.rs:66` uses — and then:

```
$ rg -n "set_permissions|0o600|PermissionsExt|peer_uid|SO_PEERCRED|getpeereid" \
      crates/operant-core/src/acp/server.rs
(0 hits)
```

`accept()` at `:58` discards the peer (`let (stream, _) = …`) and spawns
`serve_connection` on it directly. `serve_connection` reads JSON-RPC lines
(`:92`) and dispatches them through `dispatch` (`:138`) with **no
authentication layer of any kind** — `validate_request` (`:125`) checks the
JSON-RPC *framing* (`jsonrpc == "2.0"`, `id` type), not the caller.

The authority is not smaller than the session socket's — it is **larger**:

| surface | what an unauthenticated peer gets |
|---|---|
| session socket | `Prompt` / `Steer` / `Cancel` / `Approve` / `Deny` on sessions |
| ACP socket | `session/prompt`, `command`, `stop` — `dispatch` at `acp/mod.rs:430-484` |

`command` reaches `handle_command` (`acp/mod.rs:641`) → `execute_command`
(`cmd_acp.rs:154`) → `runtime.execute(command, session_id)` — an **arbitrary
command string** through the whole agent runtime. `stop` sets
`should_shutdown = true` (`acp/mod.rs:477-479`), so the same unauthenticated
peer can also **terminate the daemon**.

So the audit's own failing case is reachable *verbatim* on this box, at a
different path, today:

> "any process running as the same UID … can `connect()` the socket and send
> `WireMessage::Prompt`, which is an **unsandboxed agent turn with the full
> toolset**"

becomes: any same-uid process connects the ACP socket and sends
`{"jsonrpc":"2.0","id":1,"method":"command","params":{"command":"…"}}`, or
`stop`. The mode gate is missing too, so the default umask leaves the ACP socket
at 0755.

`bind_unless_live`'s own doc comment names this asymmetry as a *known
residual* — "the ACP server has no such lock, so two ACP starts can still race"
(`session_server.rs:61-65`) — but that sentence is about takeover, not about
authority. The mode/peer hole beside it was not carried into W6's scope.

**Fix (surgical, no redesign):** make `run_socket_server` call the two helpers
the other server already has. They are `pub(crate)`-visible today only by
virtue of being in the same crate, but both are private (`fn`, not `pub fn`) —
so either widen them to `pub(crate)` and call
`tighten_socket_permissions(path).await?` right after `bind_unless_live`, plus
the `authorize_peer(&stream)` block from `session_server.rs:135-143` verbatim
inside the accept loop, or move the pair into a small shared
`socket_auth` module. The second is better: two copies of a uid check is how
these drift, which is the same argument `run.rs:322` makes for the reasoning
helper.

**Rate: CONFIRMED FIXED for the named file; NOT FIXED for the named finding at
the class level.**

### RT-Z30-B — `peer_uid` is `cfg(target_os = "linux")`; `authorize_peer` is `cfg(unix)`. On macOS/BSD the gate is silently ABSENT, not enforced. **MEDIUM** (HIGH if anyone runs this on a Mac)

The two functions are gated differently:

```
session_server.rs:213   #[cfg(target_os = "linux")]  fn peer_uid(...)
session_server.rs:246   #[cfg(unix)]                 fn authorize_peer(...)
session_server.rs:247   authorize_uid(peer_uid(stream)?)      // calls the linux-only fn
```

`authorize_peer` is `cfg(unix)` and its body unconditionally calls
`peer_uid`, which exists only on Linux. On `aarch64-apple-darwin` this is a
**compile error, not a no-op** — but the *intent* recorded in the doc comment
is the dangerous part:

> `session_server.rs:209-212`: "macOS/BSD: `getpeereid` (not yet wired — see
> [`authorize_peer`])."

"Not yet wired" describes a state the code does not implement. The comment at
`:264-271` then asserts the non-unix arm is fine ("there is no peer credential
to read"), which is true for Windows and **false for macOS**, where
`getpeereid(3)` exists and is a two-line `extern "C"` binding.

I could not compile-check this locally (no macOS SDK cross-toolchain on this
box; `cargo check --target aarch64-apple-darwin` dies in `ring`'s build script
with `unrecognized command-line option '-arch'`, before reaching this crate).
**ASSERTED, not measured:** that the macOS build fails rather than degrades.
What *is* measured is the cfg mismatch itself (`rg` above) and the comment's
claim that the non-unix arm loses nothing.

**Why this matters beyond portability:** the failure mode of a *silent* gate is
worse than a compile error. If someone later "fixes" the macOS build by
changing `#[cfg(target_os = "linux")]` on `peer_uid` to `#[cfg(unix)]` and
stubbing it, the gate becomes a permanent no-op on every BSD — which is exactly
the shape of the original Z30 finding (a gate that looks present and enforces
nothing). The comment currently invites that fix.

**Fix:** wire `getpeereid` under `#[cfg(target_os = "macos")]` /
`#[cfg(any(target_os = "freebsd", target_os = "openbsd", target_os = "netbsd", target_os = "dragonfly"))]`,
or — the honest minimum if no Mac is available to test on — gate
`authorize_peer` on `cfg(target_os = "linux")` too, so an unsupported platform
**fails closed at startup** (the same posture `tighten_socket_permissions` now
takes) instead of compiling into a server whose comment says it is protected.

**Rate: CONFIRMED (cfg mismatch measured); ASSERTED (the macOS outcome).**

### RT-Z30-C — the uid is compared against the daemon's OWN uid, not a config allowlist. Root always connects; the trust model is uid-equality and nothing else. **MEDIUM** — deliberate, but the doc does not say so

Measured against the shipped code (`session_server.rs:252-262`):

```rust
fn authorize_uid(uid: u32) -> std::io::Result<()> {
    if uid == own_uid() { Ok(()) } else { Err(PermissionDenied…) }
}
```

and `own_uid()` is `libc::getuid()` (`:201-205`). Consequences, each verified by
reading the branch:

- **Root (uid 0) can always connect, and is not special-cased.** If the daemon
  runs as root, `own_uid() == 0` and only root passes. If the daemon runs as
  uid 1000, uid 0 is refused — the test at `:429` deliberately picks the
  "foreign" uid as `0` for a non-root process, so this is pinned.
- **There is no allowlist and no config key.** `rg -n "peer_uid|SO_PEERCRED"`
  over `crates/operant-core/src/config.rs` → 0 hits. The operator cannot say
  "trust uid 1001" without editing source.
- **A same-uid process in a different user namespace is REFUSED, not
  admitted** — which is the safe direction — **provided** the socket is not
  bind-mounted in. If it *is* bind-mounted (the documented fleet shape:
  "a process running as the same uid but a different container/user namespace,
  where the socket is bind-mounted"), the kernel reports the *namespace-
  relative* uid. Two containers both mapping their root to host uid 1000
  produce peer uid 1000 in both, and the check **passes** for a process that is
  not the operator. This is inherent to `SO_PEERCRED`, not a defect in the fix —
  but it means the gate does **not** close the fleet's own multi-agent threat
  model in the containerised case, and no doc in this tree says so.
- **TOCTOU: none.** `SO_PEERCRED` is fixed at `connect()` time and the kernel
  copies it into the socket; the peer cannot change it afterwards, and the
  server reads it on the accepted fd before any `read`. The check and the use
  are on the same fd with no intervening await (`:135-143` is straight-line
  code between `accept().await` and `tokio::spawn`).
- **No TCP fallback exists** for the session socket — `rg -n TcpListener` over
  `crates/operant-core/src/gateway/` finds only the webhook/slack surfaces.
  The socket is `tokio::net::UnixListener` and nothing else.
- **Error path is graceful.** A refused peer gets `drop(stream)` and `continue`
  (`:141-142`); the client's `read_line` returns 0 bytes and
  `SessionClient::connect` returns `"the server closed the connection"`
  (`session_client.rs:59`) — not a hang. My probe confirmed the accept path is
  non-blocking for other clients.

**Fix:** none required for correctness. **Documentation:** say in
`run_session_socket`'s doc that the gate is *uid-equality*, that root is inside
the boundary, that there is no allowlist, and that a bind-mounted socket
defeats it — because the current doc at `:102-116` claims the gate closes
"the same-uid process" case in general, and in the containerised fleet that
claim is false.

**Rate: CONFIRMED WORKING AS IMPLEMENTED; the doc overstates its reach.**

### RT-Z30-D — the Z30 regression test is NOT mutation-shaped. Removing the gate from the accept loop breaks ZERO tests. **HIGH** (this is the remediation's own charter rule 4: "a fix without a test is not accepted")

The charter's acceptance for W6 said: *"NEW test for the socket: if the platform
permits the check, a foreign-uid connection is refused (if you cannot construct
a foreign-uid peer in a test, say so explicitly rather than faking it)."* The
worker said so explicitly, at `session_server.rs:414-419`:

> "What this does NOT cover, stated plainly: it drives the COMPARISON, not the
> credential read. Reading `SO_PEERCRED` needs a second process running as a
> DIFFERENT user on the other end of the socket, which a unit test in this
> process cannot arrange … The syscall path itself is therefore unexercised by
> tests and rests on the code + the kernel contract."

That is honest and correct **as far as it goes**. What it does not say is that
the consequence is worse than "unexercised": because the shipped tests only
exercise `authorize_uid` (the pure comparison) and a *same-uid* end-to-end
connection, **the production call site can be deleted and every test still
passes.** Measured:

```
# MUTATION B: removed the entire `if let Err(refused) = authorize_peer(&stream)`
# block from the accept loop (session_server.rs:135-143), leaving the helper
# and both unit tests untouched.
$ cargo test -p operant-core --lib -- session_server
test session_server::tests::a_peer_with_another_uid_is_refused_and_our_own_is_accepted ... ok
test session_server::tests::a_real_same_uid_client_is_authorized_and_served ... ok
test session_server::tests::the_session_socket_is_0600_after_startup ... ok
test session_server::tests::a_line_the_server_cannot_read_is_answered_with_an_error ... ok
test session_server::tests::a_live_socket_path_is_refused_not_taken_over ... ok
test session_server::tests::two_clients_on_one_session_see_the_same_turn ... ok
test session_server::tests::the_session_socket_serves_two_clients_on_one_process ... ok
test result: ok. 7 passed; 0 failed
```

Seven green with the gate deleted. The control mutation confirms the suite *can*
fail — `authorize_uid(uid) || uid == 0` breaks
`a_peer_with_another_uid_is_refused_and_our_own_is_accepted` immediately — so
this is not a broken test harness, it is a **missing call-site contract**.

The repo already has the idiom that closes this: a source-grep contract, used
four times in `run.rs` alone (`:5696`, `:5705`, `:5404`,
`overflow_typed_tests`). A Z30 twin is five lines:

```rust
#[test]
fn the_accept_loop_authorizes_every_peer() {
    let src = include_str!("session_server.rs");
    let gate = format!(".{}", "authorize_peer(&stream)");
    assert_eq!(src.matches(&gate).count(), 1,
        "the accept loop must gate every connection — a deleted gate is Z30");
}
```

**Why this is HIGH and not LOW:** the charter's own definition of done makes a
test the acceptance criterion, and the sibling Z16 lane's test *is*
mutation-shaped (I confirmed: reverting its call site fails 3 of 5). So this is
not a stylistic preference — it is the one lane that did not meet the bar the
other lane met, on a HIGH-severity finding whose whole content is "a gate that
looks present and enforces nothing". The fix's code is correct; the fix's
*proof* is not.

**Fix:** add the source contract above (and ideally a `#[cfg(target_os =
"linux")]` integration test that forks a child with `setuid(1001)` and asserts
the connection is dropped without a reply — `libc::fork` is already a
dependency, so this is not blocked, only untested).

**Rate: NOT FIXED (test does not pin the fix).**

### What Z30 got right — worth recording

1. **The gate is on every connection, before any read.** `authorize_peer` is
   inside the `loop` at `:124`, not before it, and the `drop(stream); continue`
   at `:141-142` happens before `serve_client` is spawned. An unauthorized peer
   cannot read its own `HelloOk`, let alone a prompt. This is the single most
   important property and it is correct.
2. **The uid is the right uid.** `getuid()` (real uid), not `geteuid()` — so a
   setuid-root helper cannot launder a foreign caller. The test at `:429`
   deliberately picks `0` as the foreign uid for a non-root process, which pins
   "root is not special-cased".
3. **The 0600 gate is fatal, and that is the right direction.**
   `tighten_socket_permissions` at `:121` returns `Err` before the accept loop.
   I measured it: a socket bound under a deliberately `chmod 0777` directory
   comes out `0600` (`ROW mode_of_socket_bound_under_0777_dir = 0600`), and
   `SessionClient::connect` (the real client `operant chat --attach` uses)
   still connects and is served
   (`ROW client_connect = OK sessions=["probe"]`, `ROW prompt_served = 1`).
   The old code logged a warning and served; the new code refuses to serve. The
   doc at `:164-170` is honest that the caller only logs the error — that is a
   real gap in *escalation* but not in *protection*.
4. **No TOCTOU, no lock held across the syscall, graceful error path.** All
   three verified above.

---

## Z16 — compression-arm reasoning-effort rejection

### RT-Z16-A — the fix is real and correctly shared by BOTH named arms; the audit's own label is wrong. **CONFIRMED FIXED / misfiled finding**

`ae9a27cf` calls this `Z16`. It is not `Z16`. The audit's actual `Z16`
(`Z-coordinator-verification.md:513`) is:

> `approval_mode` is not merely unreachable, it is a hard config-load error …
> `[agent] approval_mode = "manual"` in `operant.toml` is not silently ignored —
> `deny_unknown_fields` makes it a parse error that fails the config load.

and it is **still open at this revision**:

```
$ rg -n 'approval_mode' crates/operant-core/src/config.rs
(no hits — the field is still absent)
```

The compression-arm finding is **index item 10** (`00-INDEX.md:280`, "ref Z16",
lane A's `A5` at `A-agent-loop-and-dispatch.md:132`). Two different findings
share the label `Z16` because the index used a shorthand ref that collided with
the coordinator's numbering. This is a ledger-hygiene defect, not a code defect,
and the sibling RT-4 doc already logged it — repeated here because *this* lane's
scope is exactly the two findings where the collision matters.

**Rate: CONFIRMED FIXED at the named mechanism (index-10 / A5); the finding
numbered `Z16` in the audit remains open and unclaimed.**

### RT-Z16-B — the fix is NOT "both arms of the request ladder". There is a THIRD `chat_streaming` consumer that rebuilds with `fitted_request` and has no reasoning rung — the mid-stream drop replay. It re-sends the rejected field. **HIGH** (measured)

`retry_without_reasoning_effort` is called from exactly two places, and the
source contract at `:5696` asserts exactly two:

```
$ rg -n "retry_without_reasoning_effort\(" crates/operant-core/src/agent/run.rs
330:    async fn retry_without_reasoning_effort(     # the definition
1924:                            .retry_without_reasoning_effort(   # streaming arm
2325:                            .retry_without_reasoning_effort(   # non-streaming arm
```

But `chat_streaming` has **three** call sites in `run()`, and `fitted_request`
has five:

```
$ rg -n "self.client.chat_streaming\(|let retry_request =" crates/operant-core/src/agent/run.rs
1886   self.client.chat_streaming(request)                       # establishment
1934   self.client.chat_streaming(retry_request)   [rung 1 — reasoning helper]
1969   self.client.chat_streaming(retry_request)   [rung 2 — compression]
2020   self.client.chat_streaming(...)             [rung 3 — credential rotation]
2219   self.client.chat_streaming(retry_request)   [rung 4 — MID-STREAM DROP REPLAY]
```

The fourth is the hole. At `run.rs:2114-2221`, when `process_stream` returns an
error **after the stream established** (a provider that closes the SSE body
mid-read, or yields an error chunk), the loop classifies, decides to retry, and
then:

```rust
2214  let tools = self.tools_for_turn(&messages).await;
2215  let retry_request =
2216      self.fitted_request(&self.effective_model(), &messages, tools);
2217  stream = self
2218      .call_with_loop_timeout(
2219          self.client.chat_streaming(retry_request),
2220      )
2221      .await?;
```

`fitted_request` (`compress.rs:719`) re-reads `self.reasoning_effort`
(`:739-744`) and attaches it **unless** `effort_unsupported_for(model)`. This
path never calls `mark_effort_unsupported`, and it is *not* downstream of a
`retry_without_reasoning_effort` — it is a sibling rung on the same `Err(e)`
arm, reached when the classification is `should_compress == false` and
`retryable == true`.

Measured with a purpose-built probe
(`crates/operant-core/examples/zzz_rt4_z16_drop_probe.rs`): a stream that
establishes, emits one chunk, then yields a `reasoning_effort`-shaped error
chunk. Result:

```
ROW stream_drop_replay_after_rejection = is_ok=true calls=2 elapsed_ms=20
     seen=[Field("high"), Field("high")]
ROW replay_carried_effort = Field("high")
```

Call 0: `Field("high")` — the provider rejects it. Call 1: **also
`Field("high")`** — the drop replay re-sent the rejected field verbatim, and
because the replay is *inside* `process_stream`'s error handler, the
`ReasoningParamRejected` classification never gets a chance to run the
reasoning rung at all. The mark is never set, `reasoning_param_attempted`
stays false, and the field rides every subsequent replay.

Contrast with the same probe on the establishment path (`stream: false`,
`Some(400)` on the first call): `seen = [Field("high"), None]` — the fix works
there. And the streaming establishment path:
`ROW streaming_reasoning_reject_then_ok = seen=[Field("high"), None]` — also
fixed. The gap is specifically the *drop replay*, which the audit's finding
("the compression path re-sends the rejected parameter") does not name but which
is the same class in the same function.

**Why the shipped tests cannot see this.** All five Z16 tests drive the
non-streaming arm (`stream: false`) with a client whose `chat_streaming` is a
stub that returns a fixed stream — so the drop-replay branch is never entered.
`both_arms_call_the_shared_reasoning_param_helper` (`:5693`) asserts two call
sites; a third consumer that *should* call the helper and does not is exactly
what a "count == 2" assertion cannot express.

**Fix:** the drop replay at `:2215` should consult the same helper before
rebuilding. The clean shape is to make the *rebuild* go through one function
rather than making every call site remember:

```rust
// in the drop-replay arm, replacing 2214-2221:
let retry_request = self
    .retry_or_rebuild_request(&retry_state, &messages)   // NEW
    .await;                                             // Option<ChatRequest>
```

where `retry_or_rebuild_request` is `retry_without_reasoning_effort(...)` OR
`fitted_request(...)`, so a third arm cannot be added without inheriting the
rule. That is the same argument `run.rs:322-328` already makes ("two copies of
one retry rule is how the arms drifted apart") — the remediation applied it to
two of the three arms.

**Rate: NOT FIXED (third consumer measured re-sending the field).**

### RT-Z16-B2 — the grace call is a FOURTH `fitted_request` producer with no reasoning rung either. It is reachable and measured. **MEDIUM**

`attempt_grace_call` (`run.rs:442`) is the budget-exhausted wrap-up. It builds:

```
461  let grace_request =
462      self.fitted_request(&self.effective_model(), &grace_messages, Vec::new());
```

then `chat`/`chat_streaming` at `:469`/`:474`. No `mark_effort_unsupported`, no
`reasoning_param_attempted`, no retry-without. It is only reached after the
iteration budget is spent (`run.rs:1543-1558` → `wrap_up_or_poke` `:798`), so
it is not on the hot path — but it *is* reached on every turn that exhausts its
budget against a provider that rejects reasoning, and in that case it sends the
rejected field **once, with no recovery**, fails, and the turn returns
`Err(Error::MaxIterationsExceeded { max })` (`:616-618`) — i.e. the grace call
fails *and* the real error is replaced by a generic budget error.

Measured with `crates/operant-core/examples/zzz_rt4_z16_probe.rs` (a provider
that always answers `finish_reason = "length"`, `max_iterations = 1`):

```
ROW grace_path_result = is_ok=true
ROW grace_path = calls=4 seen=[Field("high"), Field("high"), Field("high"), Field("high")]
```

Four requests, **every one carrying `Field("high")`** — the grace path built
all of them through `fitted_request`, and no arm ever stripped the field.
(Note: this probe used a "length" provider, so the grace call *succeeded*; a
provider that rejects the field makes the grace call fail and the turn end on
`MaxIterationsExceeded`.)

**Fix:** `attempt_grace_call`'s request build should be
`self.reasoning_effort.read()…filter(|_| !self.effort_unsupported_for(model))`
— which is what `fitted_request` already does internally — plus, on a
`ReasoningParamRejected` from the grace call, one `mark_effort_unsupported`
before the loop retries. The cheapest correct shape is to route the grace build
through the same `retry_or_rebuild_request` helper proposed in RT-Z16-B.

**Rate: NOT FIXED (measured, lower severity because it is budget-exhausted
only).**

### RT-Z16-C — the mark is PER-AGENT, not process-global; it does not reset on a model change or a new turn. Correct for the multi-session case, permanently disables reasoning for a model that later accepts it. **MEDIUM** (LOW on a single-model deployment)

The suppression set is a field on `OperantAgent`, not a static:

```
agent/mod.rs:617   unsupported_effort_models: std::sync::Mutex<HashSet<String>>,
agent/builders.rs:120, :235   initialised empty per agent
agent/events.rs:875-879       mark_effort_unsupported  -> set.insert(model.to_string())
agent/events.rs:882-886       effort_unsupported_for   -> set.contains(model)
agent/compress.rs:744         .filter(|_| !self.effort_unsupported_for(model))
```

Verified answers to each sub-question:

- **Per-agent or process-global?** **Per-agent** (`builders.rs:120`, `:235` —
  two separate builders, both `Mutex::new(HashSet::new())`). My probe confirmed
  it empirically: agent A saw a rejection, agent B (built after) still attached
  the field — `ROW scope_isolated = agentA=[Field("high"), None] agentB=[Field("high")]`.
  So **two concurrent sessions on different slots cannot wrongly suppress each
  other's reasoning.** (Caveat: a slot is `Arc<OperantAgent>`, and
  `gateway_runner.rs:3524` hashes a session key to a slot — so two chats that
  hash to the *same* slot share the set. That is by design for the slot's
  conversation, and the set is keyed by *model name*, so it is still correct
  across chats that use different models. It is only wrong for two chats that
  share a slot and a model but have different provider *endpoints* — see the
  last bullet.)
- **Does it reset on a new turn?** **No.** `run()` does not clear the set;
  `rg -n "unsupported_effort_models" ` shows no `clear()` anywhere. A model
  marked at turn 1 stays marked for the life of the agent.
- **Does it reset on a model change?** **No.** `set_model`
  (`events.rs:624-631`) retargets `model_override` and touches nothing else;
  `set_reasoning_effort` (`events.rs:831-837`) writes the effort cell and does
  not clear the set. So `/model <a-model-that-rejects>` followed by
  `/model <a-model-that-accepts>` leaves the second model **permanently
  effort-less** until the process restarts.
- **Is the key right?** It is the **model name string**, not the
  (model, endpoint) pair. Two different providers can serve the same model
  name; one rejects `reasoning_effort` and one accepts. Marking the name marks
  both. This is the same "identity is a string" class the repo elsewhere fixes
  with newtypes.
- **Could two concurrent sessions share the flag and wrongly suppress?**
  Only through a shared slot (above) or a shared model name across endpoints.
  Not a process-global, so the strong form of the concern does not apply.

**Why this is not merely theoretical:** `/effort` is a live operator command
(`gateway_runner.rs:1411-1454`, TUI at `adapter_types/tui_app.rs:2261-2274`).
The natural recovery from "reasoning is broken on this endpoint" is
`/effort off` then `/effort high` on a *fixed* endpoint — and the second
command silently does nothing, because the set still says unsupported. The
operator sees no notice: the `AgentEvent::Notice` at `run.rs:345-348` fires
only at the moment of the mark, not at every subsequent suppression.

**Fix:** clear the model's entry when the operator sets an effort
(`set_reasoning_effort` should `unsupported_effort_models.remove(&model)` for
the *current* model), and/or key the set on `(model, base_url)` instead of the
bare model string. The first is one line and closes the operator-visible
dead-end; the second is the correct fix and closes the cross-endpoint case.

**Rate: CONFIRMED WORKING AS DESIGNED for the multi-session case; the
"permanent for the session" behaviour is a real dead-end the doc does not
mention.**

### What Z16 got right — worth recording

1. **The helper is genuinely one implementation, not two copies.** `rg` shows
   exactly one `async fn retry_without_reasoning_effort` and exactly two
   `.retry_without_reasoning_effort(` call sites (`:1924`, `:2325`). The source
   contract at `:5693-5712` pins both the count and the single definition. This
   is the right shape and the right test shape.
2. **The suppression is scoped, not a global strip.** The helper calls
   `mark_effort_unsupported(&model)` on the *specific* model, and
   `fitted_request` filters on it. A provider that never rejected keeps the
   field — measured (`ROW generic_500_recovers = … seen=[Field("high")]`,
   and the shipped `a_provider_that_accepts_reasoning_is_unaffected` asserts it).
3. **The retry is one-shot and free.** `reasoning_param_attempted`
   (`turn_retry_state.rs:70`, `run.rs:342`) prevents a loop, and the helper does
   not call `consume_retry` — measured: two calls for a rejection-then-success,
   and `the_reasoning_rejection_retry_is_one_shot` fails the turn rather than
   looping.
4. **The compression arm is still reachable and still compresses.** The helper
   is the *first* arm of the `if let … else if` chain
   (`:2324` then `:2341`), so a non-reasoning overflow still falls to
   `compress_context_overflow_reporting` (`:2346`). The fix did not route the
   compression path away — it added a rung *above* it.
5. **The mutation test passes.** Reverting the non-streaming call site fails
   3 of 5:

```
failures:
  agent::run::z16_reasoning_param_tests::both_arms_call_the_shared_reasoning_param_helper
      left: 1  right: 2      # the source contract sees one call site
  agent::run::z16_reasoning_param_tests::non_streaming_retry_after_reasoning_rejection_omits_the_effort_field
      [Field("high"), Field("high")]   # the retry re-sent the field
  agent::run::z16_reasoning_param_tests::the_reasoning_rejection_retry_is_one_shot
      a second rejection ends the turn instead of looping: Ok(...)
```

The two that survive (`a_provider_that_accepts_reasoning_is_unaffected`,
`turn_retry_state_has_a_reasoning_param_guard`) are *supposed* to survive — they
are the non-regression and the state-shape checks.

---

## What to DELETE (not rename)

Per the brief: these are removals, not renamings.

1. **`crates/operant-core/src/acp/server.rs:45-73` — the unauthenticated
   `run_socket_server` accept loop.** Delete the bare `accept()`/`spawn` and
   replace with the shared `socket_auth` module (see RT-Z30-A). The function
   itself stays; the *unguarded loop* goes.
2. **`session_server.rs:209-212` — the "macOS/BSD: `getpeereid` (not yet
   wired)" comment.** It describes a state the code does not implement and
   invites a stub that would make the gate a no-op on every BSD. Delete the
   comment and either wire `getpeereid` or fail closed at startup (RT-Z30-B).
3. **`session_server.rs:264-271` — the non-unix `tighten_socket_permissions`
   stub that returns `Ok(())`.** It is `cfg(not(unix))`, which is Windows-only
   today, so it is not itself wrong — but its doc's claim that "nothing is lost
   that this platform ever offered" is the sentence that would let a future
   `cfg(unix)` widening of `authorize_peer` pass review. Delete the claim.
4. **`run.rs:2214-2221` — the drop-replay's direct `fitted_request` +
   `chat_streaming` pair.** Replace with the shared
   `retry_or_rebuild_request` (RT-Z16-B). This is the line that re-sends the
   rejected field.
5. **`run.rs:461-462` — the grace call's direct `fitted_request`.** Same
   replacement (RT-Z16-B2).
6. **`events.rs:624-631` `set_model`, and `events.rs:831-837`
   `set_reasoning_effort` — the assumption that neither needs to touch
   `unsupported_effort_models`.** Not a deletion of code, but of the *invariant*
   that the set is write-once: `set_reasoning_effort` should remove the current
   model's entry (RT-Z16-C).
7. **The `Z16` label on the compression-arm work in `ae9a27cf`'s commit body
   and in `docs/audit/2026-11-16-remediation-red-team/README.md`'s verdict
   table.** The finding numbered `Z16` in the audit (`approval_mode`) is a
   different, still-open finding. Delete the label; re-file under `A5` /
   index 10 (RT-Z16-A).

---

## Method note

- **Pinned revision:** `ae9a27cf` (branch `fleet-subagent-swarm-2452`;
  `git diff` against `HEAD` for `session_server.rs` and `agent/run.rs` is empty
  at audit end — every mutation was reverted from a byte-exact backup).
- **Mutations performed (all reverted):**
  - **A** — removed the `.retry_without_reasoning_effort(...)` call from the
    non-streaming ladder (`run.rs:2324-2343`). 3 of 5 Z16 tests fail. ✔ the fix
    is pinned.
  - **B** — removed the `if let Err(refused) = authorize_peer(&stream)` block
    from the accept loop (`session_server.rs:135-143`). **0 of 7** session
    tests fail. ✘ the fix is not pinned. (RT-Z30-D)
  - **C** — `authorize_uid(uid) || uid == 0`. 1 of 7 fails immediately. ✔ the
    test harness can fail.
- **Probes written (in-tree, `crates/operant-core/examples/`, not for merge,
  removed before this doc was finalised):**
  - `zzz_rt4_z30_probe.rs` — the real `SessionClient` against the real
    `run_session_socket`, under a `chmod 0777` directory.
  - `zzz_rt4_z16_probe.rs` — streaming/non-streaming/scope/grace rows against
    the real `OperantAgent::run`.
  - `zzz_rt4_z16_drop_probe.rs` — the mid-stream drop replay (RT-Z16-B).
  All three probes have been removed from the tree; the `ROW` lines they printed
  are quoted inline above, and `git status` shows no source modification.
- **Commands run (scoped, never `--workspace`):**
  `cargo test -p operant-core --lib -- session_server` (7 passed),
  `cargo test -p operant-core --lib -- z16_reasoning_param` (5 passed),
  `cargo run -q -p operant-core --example <probe>` ×3.
- **What I could NOT verify:** the macOS/BSD compile outcome (no cross
  toolchain; `ring`'s build script fails first) — ASSERTED. A foreign-uid
  end-to-end connection (needs a second uid on this box; only `ishanp` (1000)
  and root exist, and `sudo` requires a password) — so the *negative*
  end-to-end case rests on the `authorize_uid` unit test plus the code, exactly
  as the worker said.