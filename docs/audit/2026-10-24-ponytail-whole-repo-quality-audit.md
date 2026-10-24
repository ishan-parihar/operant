# ponytail whole-repo quality audit

Dated-frozen: audited `2026-10-24`; findings mark verified vs asserted.

## Scope

Whole-repo static audit (ponytail method): bugs, security, scale, missing
tests, lean. ~388K lines of Rust across 8 crates. Assumed load: one
operator, one long-running gateway daemon, low message volume. NOT audited:
the TUI (~135K lines), autonomous.rs, gateway_runner message-handling
internals, platform adapter internals beyond the webhook surface, AFT
bridge, sourcehound tool internals, org/kanban/curator/kernel subsystems,
plugins-wasm, mcp_serve.rs, dashboard frontend, memory-wire integration,
provider clients. No tests were run.

## Findings

- **[verified]** Webhook adapter defaults to `0.0.0.0:8080` with an optional
  secret — a secretless deployment is an unauthenticated command channel on
  every interface (`gateway/webhook.rs:166`, second fallback literal at
  `gateway_runner.rs:2186`; the only backstop is the optional admin list,
  `gateway/mod.rs:571`). The startup warning names the risk but ships the
  risky default. FIX: default `127.0.0.1:8080` via one shared constant.
- **[verified]** Stripe signature verification can never succeed: Stripe
  signs `t=<ts>,v1=<hmac(t.body)>`, the code compares the whole header
  against `hmac(body)` (`gateway/webhook.rs:129,152-157`) — every genuine
  Stripe event is 401'd. Slack/GitHub paths are correct. FIX: parse `t=` /
  `v1=`, HMAC over `"{t}.{body}"`, same 5-minute replay window as Slack.
- **[verified]** Cron schedules are evaluated in UTC everywhere
  (`cronjobs/schedule.rs:104`, `cronjobs/scheduler.rs:911`) and no surface
  documents it — `0 9 * * *` fires at 9 UTC silently. FIX: document UTC in
  `cmd_cron` help and `operant.example.toml`.
- **[verified]** "every N<h|d|w>" expands to a day-of-month cron step — a
  calendar rule, not a duration: "every 48h" fires Jan 31 then Feb 1;
  "every 2w" gets a 3-day gap across February (`cronjobs/schedule.rs:71-94`;
  the mapping is codified in tests). FIX: interval-shaped schedules keep
  their raw string and compute `next = last + period` at tick time.
- **[verified]** Six dependencies with zero references anywhere in their
  crate's source or tests: `icy_sixel`, `dashmap`, `schemars` (operant-cli),
  the optional `operant-tools` dep + `dep:operant-tools` feature
  (operant-cli — the real consumer is operant-core), `tokio-test`
  (operant-core dev-deps). FIX: remove from the manifests.
- **[verified]** `crates/operant-tools/src/microsoft365/` (1,365 lines, 4
  files) is git-tracked but undeclared in `lib.rs` — never compiled,
  referenced nowhere. Leftover from the RT-88 purge. FIX: `git rm -r`.
- **[verified]** Repo root carries an untracked literal `$HOME/` directory
  (658 files, 9.2 MB) — debris from an `install-skills.sh` run where the
  variable arrived unexpanded (`scripts/install-skills.sh:36`). FIX: delete
  the directory; guard the script against an unresolved `$HOME`.

Smaller observations deliberately not actioned: three `estimate_tokens`
copies (core/tools/tui — different purposes), non-constant-time admin-token
compare in the dashboard (localhost-bound), cron due-job claiming not atomic
across processes (pid-file singleton mostly prevents it), "every 30d" →
monthly-on-the-1st semantic stretch.
