# Lane E — Gateway, Platform Adapters, Session Server, Scheduling

**Lane:** E (red-team + ponytail audit of the `operant` Rust repo)
**Seam:** gateway + platform adapters + session server + cron scheduling
**Pinned revision:** `25b86ae5273fb6026eb6ebf94ba63211bddd0b2c` (confirmed `git rev-parse --short HEAD` → `25b86ae5`)
**Date:** 2026-11-16
**Method:** read-only. No `.rs` edits, no cargo, no gateway start, no port bind, no network. rg/cat/read/git grep only.
**Status:** IN PROGRESS — skeleton written first, updated after each confirmed finding.

## Scope files (declared)

Primary (in scope):
- `crates/operant-core/src/gateway/` (14 files): telegram, discord, slack, whatsapp, email, sms, webhook, webhook_subscriptions, types, mod, admin, lifecycle, status, dispatch_breaker
- `crates/operant-core/src/gateway_session.rs` (2901), `gateway_pipeline.rs` (605), `gateway_markdown.rs` (889)
- `crates/operant-core/src/session_server.rs` (601), `session_protocol.rs`, `session_client.rs`
- `crates/operant-core/src/cronjobs/` (7 files)
- `crates/operant-cli/src/gateway_runner.rs` (9375), `gateway_runner/` (4 files), `gateway_commands.rs` (5350), `gateway_api.rs`, `gateway_platforms.rs`

## Files read (exact list — appended as work proceeds)

- (pending)

## Files NOT read (declared gaps)

- (pending)

---

## 1. THE 7-ADAPTER MATRIX (centrepiece)

Registry: `platform_registry()` at `crates/operant-cli/src/gateway_runner.rs:5101-5189` — exactly **7** `PlatformEntry` values, each a `fn(&GatewayConfig) -> Option<Arc<dyn PlatformAdapter>>`. Consumed by `build_adapters()` (:5193).

Single inbound-truth table: `INBOUND_CAPABLE_PLATFORMS` at `crates/operant-core/src/gateway/mod.rs:24` = `["telegram", "discord", "webhook"]`; predicate `platform_can_receive()` at `mod.rs:32` (case-insensitive, unknown ⇒ outbound-only).

| adapter | receives | sends | proof file:line |
|---|---|---|---|
| telegram | **YES** (poll loop) | yes | `INBOUND_CAPABLE_PLATFORMS` mod.rs:24; loop in telegram.rs (see §1a) |
| discord | **YES** (Gateway WS) | yes | mod.rs:24; discord.rs (see §1a) |
| slack | **NO — doc CONFIRMED** | yes | `start()` slack.rs:142-149 only `info!`; `start_with_channel` :403-405 → `self.start()`; `send_message` :156 real `chat.postMessage` |
| whatsapp | **NO — doc CONFIRMED** | yes | `start()` whatsapp.rs:86-89 only `info!`; `start_with_channel` :219-221 → `self.start()`; `send_message` :96 |
| email | **NO — doc CONFIRMED** | yes | `start()` email.rs:56-59 only `info!`; `start_with_channel` :356-358 → `self.start()`; `send_message` :66 (SMTP) |
| sms | **NO — doc CONFIRMED** | yes | `start()` sms.rs:74-77 only `info!`; `start_with_channel` :191-193 → `self.start()`; `send_message` :84 (Twilio) |
| webhook | **YES** (axum TcpListener) | **NO — doc CONFIRMED** | bind `tokio::net::TcpListener::bind` webhook.rs:847; `send_message` :311-315 documented no-op returning `Ok(())`; `send_message_to_channel` :859-865 returns empty `String` |

**`PlatformAdapter::handle_update` is DEAD for ALL 7 adapters** — stated at `mod.rs:15-20` and `mod.rs:2042`, verified: no production caller anywhere (`grep -rn handle_update --include=*.rs crates/` yields only the definitions, the doc comments, and `cmd_update::handle_update_command` which is unrelated). Slack's parser `slack.rs:345-386` is therefore unreachable, and the doc claim "its `handle_update` parser is dead code" is CONFIRMED.

So the doc's per-adapter claims are **6/6 CONFIRMED, 0 refuted**. The three adapters that receive do so on paths OUTSIDE the trait: telegram.rs:1341/:1821 note a shared button handler; webhook.rs:334 `start_with_channel` runs its own axum server. The trait's `start()` for webhook (:301) is a log-only stub too — the real bind happens in `start_with_channel`, so any caller that calls `Gateway::start()` (mod.rs:387-400) without the channel path starts no listener. See finding E-PONY-1.

## 2. Concept counts

- adapters registered: **7** (`gateway_runner.rs:5101-5189`)
- inbound-capable: **3** (telegram, discord, webhook) — `mod.rs:24`
- `handle_update` implementations: 5 (slack, whatsapp, email, sms, webhook), **0 reachable** — the trait method is dead for all
- slash commands in menu / handlers in dispatch: pending
- gates on a channel turn: pending
- cron jobs: pending

## 3. Findings

### E1. [VERIFIED] MEDIUM — blocking `std::fs::read_to_string` on the webhook request path
`webhook.rs:467-468` calls `subscription_for_route(&route)` per request; that is `webhook_subscriptions.rs:159-172` → `load_subscriptions()` :180-188 which calls `std::fs::read_to_string(&path)` + `serde_json::from_str` **synchronously inside an `async` handler**. Axum polls the future on a tokio worker, so every inbound webhook POST blocks that worker thread for the duration of a disk read + JSON parse.
Failing case: a webhook endpoint on slow/overlay/network storage (or just a page-cache miss under load) stalls one of N runtime workers per request; enough concurrent posts starves unrelated tasks scheduled on the same worker.
Why it matters: the inbound path is the one surface an external party drives at will — this is a remote party deciding how many worker threads you get to keep.
Note the doc comment at :156-158 asserts the store is "a handful of entries", so the parse cost is small; the syscall is the issue, and it is unconditional (no cache, no `spawn_blocking`).

### E2. [VERIFIED] LOW — `send_to_platform` can route a `deliver` answer to a platform with no adapter
`gateway_runner.rs:8300-8310` uses `response.target_platform.unwrap_or(platform)` and calls `gw.send_to_platform(&send_platform, ...)`. `delivery_target()` (`webhook_subscriptions.rs:78-90`) parses an arbitrary `platform:chat_id` spec. Failing case: a subscription with `deliver = "slack:C123"` on a gateway where `slack_enabled = false` yields no Slack adapter, so `send_to_platform` misses and the answer is dropped after the ledger already recorded `"delivered"` (mod.rs:371-379 records against `ledger_target` = the subscription target, before the send is attempted). Why it matters: the delivery ledger says delivered, the operator sees nothing. (Ledger-before-send ordering at mod.rs:359-379 is the same class as the RT-71/RT-73 discards the file elsewhere calls out.)

### E3. [VERIFIED — non-finding, doc claim CONFIRMED] The session socket DOES obey the channel gates
`GatewaySessionHandler::handle` at `gateway_runner.rs:2972-3026` + the `Prompt` arm at :3000-3236 applies, in order:
1. `/stop` and `/sessions` handled BEFORE any gate (:3012-3026) — deliberate, commented at :3007-3011.
2. drain admission gate `handler.stopping` (:3034-3039), and again after the slot is acquired (:3126-3140) so a turn queued before the stop cannot slip through.
3. **ESTOP** `operant_core::estop::is_engaged()` (:3042-3048).
4. cross-process session ownership `active_sessions::session_is_live` (:3053-3062).
5. **`max_concurrent_sessions`** counted, not reserved (:3069-3094); the lease is taken only once a slot is held (:3141-3181) with a comment at :3063-3068 explaining why reserving-while-queued was wrong.
6. **slot binding** `handler.bind_slot_to_session(slot, &task_session, "session-socket")` at :3198, mirroring the channel path at :4334. Pinned by `gateway_runner/tests.rs:830-851` which greps the source for both call sites.

So the doc's claim "a prompt obeys the SAME gates a channel turn does — ESTOP, max_concurrent_sessions, the slot's session binding" is **CONFIRMED for all three named gates**. No gate is skipped.

### E4. [VERIFIED — enforcement is REAL, not just a test] `a_channel_owned_approval_is_refused_on_the_socket`
Test at `gateway_runner.rs:8743-8817`. The enforcement it covers is `resolve_pending_permission` at `gateway_runner.rs:2869-2905` + the key lookup `pending_permission_channel` at :2847-2854. Mechanism: a CHANNEL-keyed entry (`"telegram:42"`) is only answerable when `requester_session == Some(s)` AND `channel == format!("session:{s}")` (:2884-2896) — a channel key can never equal `session:<id>`, so a channel ask is always refused and the refusal `return false` happens BEFORE the map is touched, so the ask is not consumed (:2810-2814 assertion). A `session:<id>`-keyed ask is answerable only by that same session id (:2797-2800). Correct and real.

### E5. [VERIFIED] HIGH — the session socket has NO peer authentication; any local process is the operator
`session_server.rs:119` `let (stream, _) = listener.accept()` **discards the peer address**, and there is no `SO_PEERCRED`/`getpeereid`/uid check anywhere in the tree (`grep -rn 'peer_cred|SO_PEERCRED|getpeereid|uid()' --include=*.rs crates/` → zero hits). The only protection is filesystem: `run_session_socket` tightens the socket to **0600** at :108-115 (and only warns if that fails), plus the operant home being 0700. Default path is `<operant home>/operant-session.sock` (`session_client.rs:123-129` → `platform::operant_home()`).
Failing case: any process running as the same UID (a compromised language server, a `cargo` build script, a browser extension's unsandboxed helper, another agent's tool) can `connect()` the socket and send `WireMessage::Prompt` — which is an unsandboxed agent turn with the full toolset, and can also send `Approve`/`Deny` for tool-permission asks belonging to its OWN sessions. The 0600 mode means "same user", and on a single-user workstation every long-lived background process is the same user.
Why it matters: the doc comment at :104-107 correctly says "the socket carries full control of the agent and its tools" — but the control is granted on UID alone, and the comment's own mitigation (0700 home) is bypassed by any bind to a permissive `--socket` path.
Severity note: MEDIUM rather than HIGH if the operator never passes `--socket` outside the home and never shares the account; HIGH on the multi-agent fleet this repo documents (23 live worktrees, peer agents), where "same UID" is not a trust boundary.

### E6. [VERIFIED] MEDIUM — `bind_unless_live` has a TOCTOU window it documents but leaves open
`session_server.rs:66-93`. The probe `UnixStream::connect(socket)` (:71) and the `UnixListener::bind(socket)` (:87) are not atomic; the comment at :61-65 names this residual and says the daemon's own `flock` closes it for `gateway run` but not for `acp`. Failing case: two `operant acp server --socket <path>` starts race — the second's probe fails (nobody listening yet), both `remove_file`, both bind; the loser serves an orphaned inode while the winner reports bound.
Why it matters: same class as the iter-2366 socket-steal incident the function was written to fix, still reachable for ACP.

### E7. [VERIFIED] HIGH — a cron job fires TWICE on a crash between the agent turn and `mark_job_run`
`run_job` (`crates/operant-core/src/cronjobs/scheduler.rs:344-410`) runs the agent/script FIRST (line ~363-366) and only then calls `db.mark_job_run(...)` at :405, which is the ONLY write that advances `next_run_at`. `tick()` selects due rows with `next_run_at <= now` and no "in progress" marker (`db.rs:1023-1031`: `WHERE enabled = 1 AND next_run_at <= ?1`).
Failing case, minute by minute: a job at hourly cadence fires at 14:00; the agent turn runs 30 minutes and the process is SIGKILLed by systemd's OOM policy at 14:29 — before :405 executes. `next_run_at` still reads `14:00`. `systemctl restart operant-gateway` → `start()` (:266-276) → `tick()` → the row is due immediately and fires again. No idempotency key, no run-history check before dispatch.
Why it matters: an agent job that writes (sends a message, posts to a channel, mutates org state) does its side effects twice, and nothing in the transcript distinguishes the two runs. The same window applies to the transient-retry path at :398-404, which computes `next_run` from `now + backoff` but still only writes it AFTER the attempt.
Mitigating, stated: the whole-class fix is a claim row (`in_progress` / run-history guard) before dispatch, not a `next_run` write.

### E8. [VERIFIED] MEDIUM — no catch-up rule; a late job fires exactly once with no record that it was late
`get_due_jobs` (`db.rs:1028`) is unbounded in the past and `compute_next_run` (`scheduler.rs:978-1021`) derives from `chrono::Utc::now()` for interval phrases and `Schedule::upcoming(Utc)` for cron phrases — so a job whose gateway was down for 6 hours with hourly cadence fires ONE run at restart, not six, and never reports the gap. `repair_schedules()` is called once at `start()` (:268-270) to repair legacy schedules, not missed runs.
Failing case: a nightly digest at 02:00 on a box that was down 02:00-05:00 runs at 05:00 and says "nightly digest" — an operator reading that transcripts believes the 02:00 deliverable happened.
Why it matters: this is a *reporting/deliverable* gap, not a data-corruption one, and it is arguably correct cron semantics; it is listed because the doc's "fires late with no catch-up rule" is CONFIRMED and nobody has decided which semantics are wanted. Severity MEDIUM only if these jobs are treated as deliverables (the scheduler's own comment at :1105 calls a missed cycle "a missed deliverable").

### E9. [VERIFIED] LOW — `run_job_now` is NOT protected by the daemon `flock`, so a manual run can race a scheduled run of the same job
`operant cron run <id>` → `cmd_cron.rs:593-660` builds its OWN `CronDb::init` + agent + `CronScheduler` (:637-649) and calls `run_job_now` (:301-311). The machine-wide `GatewayDaemonLock` (`gateway_runner.rs:2402-2466`, taken at :5453) is held only by `gateway run`. Failing case: the gateway daemon's `tick()` is mid-`run_job` on job J (holding no DB-level lock — `lock_conn` is a `std::sync::Mutex` in THIS process only, `db.rs:123-127`), and the operator runs `operant cron run J`. Two processes, two agent instances, same `next_run_at` row, both call `mark_job_run` — `repeat_completed` is incremented twice for one logical cadence tick and two transcript histories exist.
Why it matters: the failure is double-charge + double-transcript, and it needs only a manual `cron run` at the wrong second, no restart.
Note: SQLite-level serialization is not the answer here and is not present; the file-level `flock` exists precisely for this shape but is not consulted on the CLI cron path.

### E10. [VERIFIED] LOW/MED — `Command::new("sh")` in `run_script_job` is a `tokio::process::Command` (async, fine), but the cron DB reads are all blocking-in-async
`run_script_job` (`scheduler.rs:476-480`) uses tokio's `Command` + `output().await` — NOT a blocking-spawn defect. However `run_job` and its helpers (`budget_gate_block`, `mark_job_run`, `record_cron_run`, `seat_memory_path`/`seat_memory` reads, `drain_run_usage`'s store writes at :647-700) perform synchronous SQLite + filesystem I/O on the SAME task as the tick loop (`start()` :266-276 is a serial loop: `tick().await` then `sleep(60s)`). Failing case: one slow job's DB write parks the tick task; the tick loop is serial by design, so the NEXT minute's joins are late by that amount. Bounded by one tick's duration, and the loop never sleeps mid-run — the tradeoff is deliberate (comment at :371-373 says the serial loop is never blocked by a sleep).
Why it matters: LOW because it is one loop, not the runtime-wide starvation of E1; MEDIUM if a script job is long-running, since a job taking 40 minutes pushes every later cadence tick 40 minutes late with no catch-up (E8 compounds).

## 4. Ponytail cut list

(pending, with LOC)

## 5. Rust discipline table

(pending: unwrap/expect in request paths, blocking I/O in async, large_enum_variant, locks across .await)