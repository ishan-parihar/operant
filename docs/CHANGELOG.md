# Changelog

## Release index

- [\[Unreleased\]](#unreleased)
- [\[0.2.1\] - 2026-09-30](#021---2026-09-30)
- [\[0.2.0\] - 2026-09-27](#020---2026-09-27)
- [\[0.1.4\] - 2026-07-19](#014---2026-07-19)
- [\[0.1.3\] - 2026-04-20](#013---2026-04-20)
- [\[0.1.2\] - 2026-04-20](#012---2026-04-20)
- [\[0.1.1\] - 2026-04-19](#011---2026-04-19)
- [\[0.1.0\] - 2026-04-17](#010---2026-04-17)


All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- **iter-1052 — the documented config examples for the model routes are corrected, and tested.** The docs showed a top-level `[[model_routes]]` and a `[memory] embedding_model`; both are rejected by the config, so copying them produced a config that would not load.

- **iter-1051 — `[[embedding_routes]]` is documented the same way `[[model_routes]]` was, and it did not work either.** The embedding hint now resolves, and an unresolvable one is a clear error instead of a model name sent to the provider.

- **iter-1051 — `[[model_routes]]` now resolves, or the hint is rejected with a clear error.** The documented usage ("pass `hint:reasoning` as the model parameter") sent that literal string to the provider as the model name.

- **iter-1049 — the TUI no longer says "Connected to AWS Bedrock." when nothing changed.** Five catalogue providers have no usable endpoint URL, so the agent kept the previous provider's endpoint; the switch now says so.

- **iter-1048 — `operant doctor` no longer reports a provider as "valid" that the runtime cannot serve.** Bedrock speaks AWS's Converse API and the runtime only speaks chat-completions and messages, so every request to a Bedrock configuration would have failed in the wrong protocol.

- **iter-1042 — the CLI no longer compiles the `postgres` dependency for a crate it never called.** The `memory-postgres` feature and the unreferenced `ci-all` meta-feature are removed.

- **iter-1041 — the installer no longer leaves a stale ONNX Runtime symlink.** The copy preserved symlinks, so an older `libonnxruntime.so.1` link survived and the installed binary could not start at all.

### Changed

- **iter-1043 — the orphan `operant-infra` crate is recorded with what it holds.** It has no dependents, and its message debouncer and stall watchdog are capabilities the gateway does not have.

- **iter-1040 — `default_provider_config` no longer claims three callers it does not have.** It had none; the per-provider defaults it reads are unwired, which is recorded as RT-89.

- **iter-1038 — 43 923 lines of provider, memory and session implementations are compiled into every binary and the product calls almost none of it.** The measurement and the three resolutions are recorded as RT-89; the configuration surface that silently ignores a provider or a memory backend is the part that needs the owner's decision.

- **iter-1037 — the `operant-tools` crate is a 30 860-line parallel implementation the product calls once.** The MCP subset is deleted here; the rest is tracked as RT-88.

- **iter-1037 — the superseded MCP stack in `operant-tools` is deleted.** Six modules and 3 232 lines with no production caller, pinning an older protocol version than the core's.

- **iter-1036 — the Slack adapter now says it cannot receive.** There is no Socket Mode WebSocket in the tree, so a configured Slack token let an operator believe they could receive Slack messages. The status report carries `inbound: false`.

- **iter-1035 — the shared updater picks the raw binary rather than an archive with the same marker, and it says so when a release carries no checksum manifest.**

- **iter-1033 — a JSON-RPC error from an MCP SSE server is no longer returned as a successful result.** The reader sent whichever of `result`/`error` it found, so a rejected tool call came back as the tool's output and the failure was reported as a success.

- **iter-1032 — the token estimate now sees the tool-call arguments and the reasoning.** A tool-call turn's text is empty by construction, so counting only the content estimated a 100 KB tool call at 4 tokens — which meant the eviction could not see the largest messages in the conversation.

- **iter-1031 — the HTTP and SSE MCP transports no longer advertise `sampling` and `roots`.** They cannot answer a server-initiated request — the HTTP path read one as the response to its own request and failed with "response ID mismatch", leaving the server blocked waiting for a reply — and advertising a capability the transport cannot honour is worse than not advertising it.

- **iter-1031 — a `transport = "streamable-http"` MCP server can now connect.** It was routed to the plain-HTTP client, which sends no `Accept` header (a spec-compliant streamable server answers `406 Not Acceptable`), never echoes the `Mcp-Session-Id` (so every request starts a new session), and cannot read an SSE-framed response. The plain-HTTP transport is unchanged.

- **iter-1030 — dropping an MCP stdio transport no longer leaves the server subprocess running.** `kill_on_drop` defaults to false and there was no `impl Drop`, so every transport dropped without an explicit `disconnect()` — a failed connect, a manager teardown, the agent shutting down — left a live MCP server process behind.

- **iter-1029 — a failed Telegram send is no longer reported as delivered.** Only HTTP 400 was special-cased; every other failure status fell through to a JSON parse that succeeded on Telegram's error body, and the absent message id became an empty string — so a revoked token, a blocked bot, a 429 and a 500 all looked like success. The status now decides. The `edit_message` path no longer treats every 400 as "not modified" (which silently lost answers whose text exceeded Telegram's 4096-unit limit), and a failed poll ends with an error that names the likely cause instead of looping silently every two seconds.

- **iter-1028 — the LLM compression no longer orphans a tool pair.** The compressor's HEAD boundary was a raw message count, so a `tool_use` at the head's edge and its `tool_result` in the summarized middle were separated — a malformed history that strict providers reject. The head boundary is now group-aware, and the compression removes a `tool_result` stranded after the summary (the repair function for that existed since the compression path was written, with no caller outside its own tests).

- **iter-1027 — a TUI test can no longer poison the process-global reasoning-mode cell.** The guard restores the cell on drop, so a test that panics before its manual reset no longer leaves the mode changed for every test that runs after it. That was a full-suite failure that passed in isolation.

- **iter-1026 — the LLM compression no longer orphans a tool pair.** The compressor's HEAD boundary was a raw message count, so a `tool_use` at the head's edge and its `tool_result` in the summarized middle were separated — a malformed history that strict providers reject. The head boundary is now group-aware, and the compression removes a `tool_result` stranded after the summary (the repair function for that existed since the compression path was written, with no caller outside its own tests).

- **iter-1019 — the AFT binary is now verified before it is installed, and the install is atomic.** The shared updater's checksum guard matched only the asset name `SHA256SUMS`, and `cortexkit/aft` publishes `checksums.sha256` — so the verification never ran for aft, and the updater installed a freshly downloaded binary with no checksum comparison. The first-install path (which never went through the shared updater at all) downloaded, wrote and executed a binary with no integrity check and a non-atomic write. The guard now accepts both spellings, and the first install verifies the bytes against the release's manifest before writing, and writes through a temp file and a rename. A first install now FAILS CLOSED when the manifest cannot be read — a fresh download of an executable that will be run must be verifiable.

- **iter-1018 — the MCP tool's registry name is injective again, and the claim that says so is true.** The encoding left `_` alone while encoding the illegal characters as `_<hex>_`, so `-` and the literal string `_2d_` produced the same name — the exact collision the name was supposed to prevent. Every byte outside `[A-Za-z0-9]` is now encoded as `-` plus two hex digits and the components are joined with a single `_`, so the qualified name's body carries exactly one underscore and the pair is recoverable by splitting on it.

- **iter-1017 — the Email adapter and the proxied-WebSocket path no longer panic on their TLS setup.** `rustls::ClientConfig::builder()` panics when the crate features are ambiguous, and both crypto providers were enabled (`aws_lc_rs` from hyper-rustls's defaults, `ring` from tokio-rustls's) — so every SMTP send through the Email adapter hit the panic, making the adapter non-functional, and the same call shape broke every proxied WebSocket connection. The crypto provider is now named explicitly at the call site, which works under every feature combination.

- **iter-1017 — the SMTP client no longer reports a rejected message as sent.** Every server reply was read and discarded, so a `535 Authentication failed`, a `550 mailbox unavailable` and a `552 message too large` all produced a successful send. Every step now checks the server's status code, and a rejection names the code and says the message was not sent.

- **iter-1016 — a context overflow that the compression cannot help now fails with a typed error instead of a provider 400.** The reactive overflow path compressed the conversation and re-issued the request unconditionally: if the compression could not reduce the history, the retry sent the same prompt the provider had just rejected, earned the same 400, and the turn died with a raw provider error the gateway cannot act on. The rule is deliberately narrow, because a provider overflow can be transient: the FIRST overflow is still retried, and only the SECOND one — after a compression that did not reduce the history — fails typed with `Error::ContextExhausted`. The eviction itself also gained a final trim (the recency reserve is trimmed, oldest first, when that is what is over budget) and now REPORTS whether it met the budget, instead of returning an over-budget list with no signal.

- **iter-1015 — the gateway now prunes idle sessions.** The session store's `prune_old_entries` had been in the code since it was written, and its only callers were its own tests — so every conversation the gateway ever saw stayed in the database and the in-memory map forever. The gateway now sweeps every 6 hours, and `[gateway] session_retention_days` sets the horizon (default 90 days; 0 keeps them forever, the previous behaviour).

- **iter-997 — the TUI visual corpus is now part of the local gate.** `_run.py verify` has a real exit code (1 on drift, 0 on pass), and it was in NO automated gate: `scripts/check.sh` forwards to cargo, and the corpus is a Python script. The corpus had been red for nine days and nothing ran it. `scripts/self-test.sh` runs it now, against the release binary it has already built.

- **iter-988 — the release is deployed and a third operant is cleared from PATH.** `~/.cargo/bin/operant` needed libonnxruntime 1.20.1 and the runtime has 1.24.4, so every invocation of it failed with a version error; it is moved aside (not deleted). `operant doctor` now reports two copies on PATH instead of three.

### Security

- **iter-1014 — an MCP tool's registry name is now injective.** The name was `mcp_{server}_{tool}` with `/` and `-` folded to `_`, so two different (server, tool) pairs could share a name. The registry replaces a tool on a name collision and the sync tears down by name, so a hostile MCP server could shadow another server's tool — and tearing one server down removed the other's live tool. The same expression produced names the provider rejects (a server named `my.srv` gave `mcp_my.srv_tool`, and OpenAI's strict alphabet has no `.`, so the whole request failed). The name is now built by a lossless encoding that is injective by construction and always inside the provider's alphabet: `github` + `create_issue` is `mcp_github-create_issue`.

- **iter-1009 — a webhook adapter with no signing secret now says so at startup.** The HMAC check is skipped entirely when no `webhooks_secret` is configured, so every POST is accepted and forwarded into the agent as a message — and the adapter's default listen address is `0.0.0.0`. The configuration itself is unchanged (a private-network receiver with no secret is legitimate), but the adapter now warns at startup with the address, what is accepted, and the two ways to fix it.

- **iter-998 — the dashboard's API now accepts the auth header the bundled frontend actually sends.** The frontend sets `X-Hermes-Session-Token` on every request and never sends `Authorization`; the server read only `Authorization: Bearer`, so every API call the real dashboard made returned 401 and the UI could never have worked. The token is now accepted as `Authorization: Bearer`, as `X-Hermes-Session-Token`, or as the dashboard cookie.

- **iter-996 — the dashboard's session token is no longer served on an unauthenticated route.** The index page injected the token into its HTML and required no auth, so anyone who could reach the port could read the token and then call every "protected" API route with it. The index route now serves the token-bearing page only to a request that already holds the token (the CLI's browser launch exchanges it for an HttpOnly cookie), and a request with a foreign `Host` header is refused, which blocks DNS rebinding against the loopback-bound dashboard.

### Fixed

- **iter-1012 — the credential rotation works in four more configurations it previously did not.** A rotation after a 401 or 429 stored the new key on the benched credential, so the pool would not select it and the agent answered "no available keys" with a working key in hand; the failure state belonged to the old secret and is now cleared with it. The "current key" accessor reported the literal placeholder `default` when there was no key, and the anthropic client implemented neither accessor at all — so the endpoint retarget emptied the credential pool on that provider. Changing `operant.toml` emptied the pool too, because the key it passed came from the config file, which does not apply the environment overrides, and the documented setup keeps the key in `~/.operant/.env`. The rotation's anchor — the pool credential a rotation replaces — is now refreshed when the agent loop moves the client to a different credential, so a rotation after a 401 rotates the credential in force rather than the benched one.

- **iter-1012 — the CLI and the core resolve the operant home the same way on every platform.** The CLI built the config dir from the home directory, while the core uses `%APPDATA%\operant` on Windows — so on Windows the startup `.env` loader and the per-turn `.env` reload read two different files. There is now one resolver.

- **iter-1012 — the watchdog's thread-state records are unambiguous.** The record is `tid:name:state:wchan`, and the thread name comes from `comm` — which for a Rust test thread is the module path, `tui::app::tests`. The name's colons made the four-field record ambiguous, and the test that checked the shape failed on any machine whose test threads carried a module path. The name's colons are now dots.

- **iter-1012 — the trajectory cap is the operator's choice, and the export says when it is in force.** The prune that keeps `~/.operant/trajectories` bounded deleted the oldest files past a hard-coded 200, with no way to turn it off — and `operant trajectory export` exists to collect those files as prompt-completion pairs for fine-tuning, so the prune was silently destroying the corpus that command is for. The cap is now `[agent] max_trajectories`, default 0 (keep them all), and the export reports when a cap has truncated the corpus.

- **iter-1012 — the PDF decompression bound now covers the load path and the whole document (AD-041).** The 64 MiB limit was applied to the extraction, but the document loader decodes the object and cross-reference streams eagerly and was unbounded — the largest decompression in a PDF happened before the per-page limit was consulted. The load is bounded now, the per-page budget is divided across the pages read, and a document's page count is capped at 256.

- **iter-1012 — `operant checkpoints clear` now reclaims the disk.** The command removed the database rows and left the shadow store exactly as large as it was, so a store that had reached 1.5 GB of packed objects reported "cleared N checkpoints" and freed nothing. The store holds snapshots of the working directory, so clearing it removes a rollback history and nothing else; the command now removes the store too and reports the bytes it freed.

- **iter-1012 — the PDF tool's test no longer accepts a broken port.** The test passed when the result was an error containing "no extractable", so an extractor that returned empty for every file would have passed. It now asserts the tool's output carries the text of the PDF it read.

- **iter-1011 — the checkpoint shadow store no longer snapshots itself.** The store lives inside the directory it snapshots, and its exclude list did not name it, so every checkpoint committed the store's own packed objects and each snapshot embedded the previous one's pack. On this box that had produced 11 commits with 2.68 GiB of reachable blobs, 1.21 GiB of them the store's own pack files. The store is now excluded by name and by its own absolute path, and the exclude file is rewritten on every use instead of only when the store is first created. The existing store is not reclaimed by the fix — deleting `~/.operant/checkpoints/store` is the owner's call, because it removes a rollback history.
- **iter-1010 — a checkpoint skipped for size now says so.** The size-bound skip (iter-992) recorded its reason at debug level only, and the tool's error message named two other causes — so an operator whose tree was over the limit was told to enable checkpoints that were already enabled. The message now names the size, the limit and the config key to change.
- **iter-995 — `operant logs follow` keeps working across a log rotation.** The rotation replaces the live file with a new one, and the follower tracked a byte offset into the old file: it printed nothing until 32 MB of new output had accumulated, and then it seeked into a stale offset and printed half a line. The follower now tracks the file's identity as well as the offset.


- **iter-994 — a hook that panics while building its future is reported too.** The panic-at-poll-time shape was fixed in iter-993, but `HookHandler` is `Fn(..) -> Pin<Box<dyn Future>>`, so the closure body runs at call time and that shape escaped the caught region.


- **iter-993 — a panicking hook is now reported instead of swallowed silently.** The panic was always contained, but the warning that says so was unreachable, so a hook that panicked on every turn was invisible in the logs.


- **iter-992 — the checkpoint shadow store now has a size bound, and its size limits are no longer dead config.** `max_total_size_mb` and `max_file_size_mb` were declared and never read, so nothing bounded the bytes a snapshot could take. A working tree over either limit is now skipped rather than snapshotted.


- **iter-991 — a configured `logging.log_file` is now rotated.** The file was opened in append mode with no size bound, so the gateway's log grew without bound (90 MB after about three weeks on this box). It now rotates at 32 MB and keeps two generations.


- **iter-990 — the trajectory directory no longer grows without bound.** A trajectory is written per run, so a long-lived session accumulated one file per run, and a Holon run always records. The write path now keeps the newest 200 and prunes the oldest, the way the checkpoint store bounds its snapshots.


- **iter-989 — `[client.rate_limit] max_retries = 0` no longer panics the process.** The attempt count was used raw in `for attempt in 1..=max_retries`, so a 0 made the range empty: the request was never made and the non-streaming retry loop hit an `unreachable!()`. In the gateway daemon that is a crash. A 0 now means "one attempt, no retry", which is what the setting means, and the loop's fall-through is an error rather than a panic.


- **iter-987 — PDF text extraction no longer reaches an unmaintained crate, and a decompression bomb is bounded (AD-039).** The `pdf_read` tool is ported from `pdf-extract` 0.12 to `lopdf` 0.45, which drops `ttf-parser` (RUSTSEC-2026-0192) from the dependency graph. The port also bounds each page's decompressed content stream at 64 MiB; `pdf-extract` did not bound it, so a small PDF could expand into gigabytes.



- **iter-986 — rotating a credential in `.env` now actually rotates the running agent's credential.** The gateway's per-turn `.env` reload wrote the new key into the process environment, and nothing read it back: the client captured its key when it was built and the credential pool seeded its keys at startup, so a rotated key reached nobody. The reload now reports the credential and the endpoint it found, and the gateway pushes them into the live client. A key rotated in the documented `~/.operant/.env` now takes effect on the next turn, without restarting the gateway daemon.

  This supersedes the iter-985 entry, which claimed the same thing and was wrong: it did not reach the credential pool, which re-selects the startup-seeded key before every request, and it read a CWD-relative `.env` rather than the documented `~/.operant/.env`. Both are fixed here, and the two operations that were conflated — "use this key now" and "replace the stored credential's secret" — are now separate.


- **iter-984 — a broken MCP server no longer stops operant from starting (AD-036).** One unreachable MCP server (a stale URL, a binary that is not installed, a host that is down) aborted every command that builds the tool registry, so `operant tools list` exited 1 with a raw `Connection refused` and the agent could not start at all. The failure is now recorded, logged, and shown in the `/mcp` view as an `error` status with the reason; the rest of the registry still builds. The `/mcp` view now distinguishes connected, disconnected (never tried), and error (tried and failed).


- **iter-982 — the supply-chain gate runs, and the clippy gate no longer aborts on an empty allowlist.** `cargo audit` was wired but not executed (`cargo-deny` is not installed, so the licence and duplicate-version policies in `deny.toml` stay unexecuted by design), and the clippy gate treated an empty allowlist as a failure.


- **iter-981 — the build no longer dies in bindgen when libclang is missing or stale (AD-035).** Three build scripts resolved libclang three different ways, and the one `dev-env.sh` used fell through to a directory that does not exist and only warned — so the build failed two minutes later inside `espeak-rs-sys` with a message naming neither the cause nor the fix. One resolver now owns the choice for all three scripts, a symlink whose target the machine removed is rejected rather than trusted, a missing libclang is a startup error that names the fix, and the provisioner repairs a dangling link.


- **iter-980 — the composer now draws the vim mode, and the typeahead ranks slash commands by how often you use them.** `VimMode::label` and `rank_commands` were both computed and never called: the composer never showed `NORMAL`/`INSERT`/`VISUAL`, and the recorded command usage never affected anything. The typeahead now ranks through the usage store it already keeps in memory, so a keystroke never waits on the usage file.
### Added

- **iter-954 — a Holon body's trajectory names its run and employee (AD-001).** A Holon run's operant-side trajectory carries `holon_run_id` and `holon_employee` in its metadata, in the operant home's `trajectories/` file. The kernel's `trajectory.json` keeps its shape, and body runs still write no `sessions` row.

- **iter-929 — context bust permission (plan 06 P1).** Each request gets a
  `BustPermission`, decided once in `build_messages`: `Natural` when the cache is
  cold anyway (model or system prompt changed, or the last request is older than
  `[client] prompt_cache_ttl`), `Force` at ≥85% of the window, `Explicit` after
  `/compact`, otherwise `None`. Micro-compaction, the preflight ladder, and eviction
  run only under a bust, so a request below the force line is the previous request
  plus the appended messages. A forced pass folds to the 40% soft line, so a crossing
  rewrites once. At ≥95% with nothing left to fold the turn fails with
  `Error::ContextExhausted` instead of a provider 400; the gateway compacts and
  retries once, and the TUI shows the message, which says to run `/compact`. An
  overflow records the smallest overflowing size per provider and model in the
  state table, and later builds size against it. Not changed: the context engine's
  `assemble` still runs every turn; the `/ctx-flush` command named in the plan does
  not exist.

- **iter-902 — `operant donate` command + installer star/donate CTA**
  (owner-directed integration). New `Donate` command printing the repo,
  GitHub Sponsors, and Razorpay (UPI) links; `scripts/install.sh`
  rewritten as a per-user installer (root guard with `--force-root`
  override, package-manager-aware preflight with per-distro install
  hints, star/donate CTA epilogue gated by `--no-cta`, referencing
  `operant donate`); README badge band cleaned to the CI badge.

### Removed

- **Orphaned integrations removed (iter-957): Jira, Notion, Composio, Google Workspace, Pushover, weather, git operations, LinkedIn.** No agent or gateway path registered these tools, so nothing reached them. Removed: nine `operant-tools` modules, the `operant-tools` `tokio-tungstenite` dependency, the `[notion]`, `[jira]`, `[composio]`, `[google_workspace]`, and `[linkedin]` config sections with their validation, the Jira fields of `[project_intel]`, the `"weather"` default auto-approve entry, and the unused `[linkedin.image.*]` provider structs (the live image tool reads `[image_gen]`). A stale section in `operant.toml` is now ignored, not rejected: delete it. The Holon Notion adapter (`[holon] notion_script`) is separate and unchanged.

- **Never-read CLI config fields removed (iter-943).** `crates/operant-cli/src/config.rs` drops 167
  field definitions that no code read (398 → 231), with their defaults, and 12 structs with no remaining use.
  `CliConfig` never reads a file and `operant.toml` rejects these sections, so no setting depended on them.
  Nothing changes at runtime.
- **`glm.rs` removed (iter-942).** The file was not compiled (no `mod` declaration, no caller), and
  its comment in `lib.rs` is gone with it. It remains in git history.
- **Dead TUI items removed (iter-941).** Functions, constants, and types that the all-features gate flagged as
  never used are deleted from `crates/operant-cli/src/tui/`. Names the compiler still needed are kept. The clippy
  allowlist drops from 58 to 52 entries.
- **`StoredCredential::OAuthToken` removed (iter-940).** Nothing constructed it once the device-auth scaffold
  was gone, so its `refresh` and `expires` fields were never read. `api_key_for` no longer needs a fallback arm.
- **`channel-nostr` placeholder feature removed (iter-939).** No crate enabled it, so its gated
  `NostrConfig`, impl, relays helper, and `channels.nostr` field were never compiled into a shipped build.
  The README lists `voice-wake` as the only remaining empty feature flag.
- **Memory factory removed (iter-938).** `create_memory`, `create_memory_with_storage_and_routes`,
  `create_memory_for_migration`, and `create_response_cache` had no caller outside `operant-memory`. The
  modules only they reached go too: `hygiene` (the purge pass), `audit` (`AuditedMemory`), and
  `response_cache`. The autosave-key helpers and the embedding-config resolver went with the factory, and
  so did the 19 tests that covered them. Production memory is still built by `load_memory_manager`.
- **`device_auth_dialog` scaffold removed (iter-937).** The dialog had no opener, so nothing could reach
  its states. Removed with its key handling, dialog priority, overlay entry, and corpus references
  (`excluded.json`, `schema.json`, `_validate.py`). The real OAuth device flow in
  `operant-core/src/oauth/device.rs` is unchanged.
- **`commands.rs` dead command layer removed (iter-936).** The 17 `#[allow(dead_code)]` attributes
  that hid it are gone, and the items they covered are deleted: `CommandCategory`, every
  `CommandResult` variant no caller constructs (only `Message` and `ShellCommand` remain), the
  `CommandDef` category and help fields (`CommandDef::new` now takes name and description), the unused
  `CommandRegistry` helpers, `resolve_command`, and the skill and TUI slash caches. The tests that
  covered them are removed; the resolve and execute tests now run against `CommandRegistry`.
- **TUI legacy purge — dead code removed from the `operant-cli` TUI tree (2026-10-18).**
  684 items the compiler reported dead in both the production and the test build were
  deleted, with the impl blocks, nested items, and trait-method overrides that depended
  on them. The blanket and per-item `dead_code` suppressions were stripped so the compiler
  report is visible again; `tui/mod.rs` keeps the module-level expectations from HEAD.
  Nothing live changed: both builds compile with 0 errors, and
  `cargo test -p operant-cli --bin operant` passes 1596 tests (7 ignored), the same count
  as before. Kept on purpose: `operant_ui/ui_header.rs`, the live header renderer; replacing
  it is an owner decision. The 60 `dead_code` warnings that remain (test-only
  helpers and never-constructed variants/fields) are tracked in
  `.ci/clippy-allowlist.txt` and in `docs/audit/2026-06-19-BUGS.md`.

- **iter-947 — `[display] show_reasoning` is removed; `[agent] show_reasoning` is the one key.** The CLI config had two keys for the TUI reasoning display, and `[display]` overrode `[agent]`, so the two could disagree. The TUI and the gateway now both read `[agent] show_reasoning`. A `[display] show_reasoning` key left in the CLI config is ignored. A user who set `[display] show_reasoning = false` should move that value to `[agent]`.

- **iter-949 — the unregistered `proxy_config` agent tool is removed.** Nothing registered it, and its writes went to `config.toml`, which no part of the CLI reads. Proxy settings are configured in `[proxy]` in `operant.toml`, applied at startup. The tool's test lock went with it: the web-search tests no longer need one. The selector error now names the valid service keys.

### Fixed



- **iter-978 — warnings triage: the production build of operant-cli is warning-free (AD-034).** Dead paths are deleted: the turn-diff mode (`/changes` now opens the working-tree diff, the diff viewer's `d` key no longer switches modes, and `/rollback` names `/rewind`), the unused slash wrappers, `TurnState::Connecting`, 13 unreferenced `KeyAction` variants, `add_model_usage`, `encode_image_base64`, `find_with_fallback`, `remove`, and `McpViewStatus::Error`. Error notices now render as error rows in the transcript, where they drew as plain system rows before. The clippy allowlist shrinks from 47 to 24 entries; the remaining keys are test-configuration dead code, and the open gaps behind kept items are BUGS RT-29 to RT-33.

- **iter-977 — dependency advisories cleared: `atty` (vendored `kokoro-tiny`), `rustls-native-certs` 0.8, `chacha20` 0.10.2, and `image` without EXR.** The YAML crate swap is reverted; `unsafe-libyaml`, `ttf-parser`, and `bincode` remain open or accepted (RT-26 to RT-28, AD-033).
- **iter-976 — RT-18 closed: `ndarray-npy` is patched to `zip` 9 (`vendor/ndarray-npy`), so the TTS voice file no longer pulls a vulnerable `zip` (AD-032).**
- **iter-974 — autonomous publish stages only its own paths, the review items are decided, and dead TUI code is removed (AD-027 to AD-031).** The autonomous loop stages and commits only the paths its tick made dirty, so a file the owner left uncommitted is not published. Decided: new org chats stay unbound (AD-028); the synchronous summarize rung stays (AD-029); the Telegram token filter is covered and the historic log is not rewritten (AD-030). Dead TUI code that is ours is removed, and the clippy allowlist goes from 52 to 47 (AD-031).
- **iter-973 — the diff viewer reads git off the event loop, a `[keybindings]` table no longer stops the config from loading, and the settings.json hooks menu is removed (BUGS TUI-11, RT-3, RT-14; AD-024 to AD-026).** `/diff` shows a reading state while `git diff` runs on a worker thread. `[keybindings]` is accepted and warned about until dispatch reads it. The read-only hooks menu is removed, since nothing ran the hooks it listed; `/hooks` names `operant hooks`.
- **iter-972 — `operant plugins disable` now keeps a plugin out, and the WASM plugin path is closed on four fronts (BUGS KB-10, KB-11, KB-17, KB-18, KB-20; AD-023).** `[plugins].disabled` in the config is the deny list; `operant plugins enable|disable` and the TUI plugins hub write it, and a change takes effect on the next start. The `<name>.enabled` marker files are no longer written or read. A memory plugin gets a query only with `MemoryRead` and turn text only with `MemoryWrite`. A plugin tool cannot take a built-in's name. A plugin's HTTP request is refused for loopback, private, link-local and unique-local addresses, and does not follow redirects. The memory plugin reads its tool schemas once at load.
- **iter-971 — the gateway's memory is kept for its named admins, and stored knowledge is bounded, redacted and written whole (BUGS KB-3, KB-6, KB-8, KB-12, KB-13, KB-14, KB-15, KB-16, KB-19; AD-019 to AD-022).** A gateway turn is retained, and recalls the personal bank, only for a sender named in `gateway.admins`; an empty list (the default) retains nothing from the gateway, and other senders get a bank of their own. `memory_store`, `skill_manage` and `learning_manage` ask for approval like file writes, and a background review refuses `memory_store`; the review's skill guard now runs in production (it was blind to the task-scoped origin). `memory_store` refuses oversized keys, content and tags; turns, facts and skill text are redacted, and a skill that embeds a secret value is refused. MEMORY.md, USER.md and learning SKILL.md writes are atomic; skill paths refuse symlinks. `memory stats` reports an in-memory fallback, and `memory delete` asks for confirmation like `memory clear`.
- **iter-970 — the TTS engine no longer aborts the process, and the ONNX Runtime pin serves the API `ort` needs (BUGS CR-1, CR-2, CR-3, CR-4, CR-5, CR-6; AD-016 to AD-018).** A Kokoro start that panics is a tool error: the engine starts on its own task, and the release profile unwinds. ONNX Runtime is pinned to 1.24.4 everywhere, and `install.sh` refuses a library that does not serve API 24. `operant doctor` marks a tool available only through its configuration or an env var, reports TTS as checked on first use, and lists every `operant` on PATH. The config loads once at startup, so each `[memory]` warning prints once. The deploy must copy the 1.24.4 libraries to `~/.local/lib/operant` before CR-1 and CR-2 close. A stray `xd:/` note is excluded locally, and the `github` remote refs are in sync (RT-8, RT-17).
- **iter-969 — the TUI's approval, rename, delegation, shell-out and branch rows are fixed (BUGS TUI-1, TUI-6, TUI-7, TUI-8, TUI-9, TUI-10, TUI-12, TUI-13; TUI-11 branch half; AD-011 to AD-015).** An approval prompt answers its key before any overlay. `/rename` saves the confirmed name, and a bare `/rename` starts once the list loads. A delegation whose turn is cleared is marked abandoned, and the status bar shows a running one first. A shell-out restores the terminal before it reports the child's exit, reports a failed or signalled exit as such, and frees the mouse for the child. The Ctrl+B fork tree is read off the event loop, and a database error is shown in the transcript. The git-diff load still runs on the event loop; TUI-11 stays open for it.
- **iter-968 — the gateway's budget and conversation rows are fixed (BUGS GW-1, GW-2, GW-5, GW-6, GW-7, GW-8, GW-9; AD-009, AD-010).** A budget window now sums a per-turn ledger charged to the seat the turn started under, so a rebind no longer moves spend and a window rolls. A hard cap whose usage cannot be read refuses the turn; soft mode runs. A chat's conversation id is kept on its session row: `/new` and a seat change start a fresh transcript (the old one stays reachable with `/resume <id>`), and the continuation summary is keyed by it. Cron and socialization runs set their seat as the memory scope. `/session` no longer names the premiere for an unbound chat.
- **iter-967 — Holon bodies are fenced from the supervisor's secrets and their slot, and org mode refuses a run with no policy (BUGS OS-1, OS-2, OS-3, OS-5, OS-6, OS-7, OS-8, OS-9, OS-10, OS-11; AD-006, AD-007, AD-008).** A body can no longer read the supervisor's environment, and its chair key is pinned to a value that matches nothing. A signalled supervisor kills its body's whole process group. The kernel's run status decides whether a slot is over, so a reclaimed run is killed rather than read as a clean self-submit. A timed-out native call takes its kernel child with it, and its argv keys must be plain flag names. A TERM-trapping body group gets SIGKILL after a grace period. `supervise --loop` resets its back-off per pass. A Holon run whose `policy.json` is missing refuses to start. The Holon kernel (local commit `992b08f`) re-offers a chair notice until it is delivered, and refuses a driver placeholder that is not a plain token. Residual: a same-uid body can still read the operator's chair key file directly until the O-11 runner isolation lands.
- **iter-966 — `/resume` switches the agent, a failed shell-out restores the terminal (TUI-2, TUI-6 in part).** `/resume` used to load the transcript without retargeting the agent, so the next prompt went to the old session. It now sets the same pending switch as the branch browser, and the agent and the transcript move together (`tui/app/key_handling.rs`, `tui/app/commands.rs` tests). The separate load channel is removed, and the switch resets the header title. A shell-out whose binary fails to spawn now restores the terminal before it returns the error (`tui/adapter_types/tui_app.rs`). That last fix has no test yet.

- **iter-965 — an unbound chat no longer keeps the previous chat's employee seat (GW-3).** The agent is shared across chats, and the seat was set only by a bound turn, so an unbound turn after `/new` ran under the old employee's policy and budget gate. Every turn now clears the seat before the binding lookup (`agent/events.rs` `clear_seat_id`, `gateway_runner.rs`). The memory scope has the same gap when no session row matches; it stays open under KB-8.

- **iter-964 — agent creation never overwrites, agent descriptions round-trip, and `/rewind` confirmation cancels on any other key (TUI-3, TUI-4, TUI-5).** Creating an agent refuses a path that already exists, including a dangling symlink, and names the file (`tui/agents_view.rs`). A multi-line description is saved as one JSON-quoted YAML scalar and read back whole (`tui/agents_view.rs`). Any key other than y or n cancels a pending `/rewind <n>` before the key is handled (`tui/app/key_handling.rs`).

- **iter-963 — observe posture denies `holon_skill_invoke`, and `chair notify` fails when nothing is delivered (OS-4, OS-5).** A `skill` invoke writes the org ledger, so it is now in `OBSERVE_HOLON_DENY` (`operant-os/src/policy.rs`). `chair notify` returned 0 after every send failed. The send loop is now `deliver_to_chairs` (`cmd_org.rs`), which returns an error when `sent == 0 && failed > 0`. The kernel's missing retry (OS-5, Holon repo) is still open.

- **iter-962 — skill writes are screened, skill text is escaped, and queued memory writes keep their bank (KB-4, KB-5, KB-6 in part, KB-7, KB-9, GW-4).** Every skill write (create, edit, patch, write_file) runs the security scan, and a refused write is rolled back (`skills_tool.rs`). A skill's name and description are XML-escaped in the frozen system prefix (`agent/mod.rs`). Recalled memory cannot close its `<memory_context>` wrapper and is labelled as data; retention for untrusted channels is still open (TODO.md Review). The memory sync job carries its bank from enqueue, so a scope change after queueing does not move the write (`memory_provider.rs`, `memory_wire.rs`). A Holon run's `learning_manage` delete or edit is recorded as an annotation on the run and does not touch the personal graph (`learning_mutation_tool.rs`).

- **iter-958 — the knowledge-base tools stay inside their directories (KB-1, KB-2).** `learning_manage` delete and edit, and `skill_manage` write, delete, and view, used a model-supplied name as a path with no containment check, so `../../.ssh` moved a directory outside the skills tree. Skill ids and names are now plain path segments, and a `category` gets the same rules as a `name`. The approval gate and the security scan on skill writes are not changed here (KB-3, KB-4).

- **iter-954 — `/session <employee>` checks the Holon roster in org mode (AD-004).** In org mode an id that is not on the roster is refused, and nothing is bound. An unreadable roster is refused too. Personal mode binds as before. `/tasks` now describes the agents menu it opens; it said "autonomous task ledger".

- **iter-955 — a bearer token that is not header-safe is an error, not a panic (RT-20).** The Home Assistant `get_headers` and Spotify `headers` builders now return `Result`, and each of their callers returns `ToolResult::error`. The LinkedIn `api_headers` returns `anyhow::Result`, and its request path, 401 retry, and image upload propagate it. A token with a control character from config now fails the tool call with a message instead of aborting it.

- **iter-953 — the OAuth PKCE verifier comes from an OS-seeded CSPRNG on every target (RT-19).** On non-Unix targets the verifier came from a pseudo-random state seeded with the clock, so it could be predicted from roughly when it was generated. It now draws 64 bytes from `rand::thread_rng()` everywhere, and the `/dev/urandom` read and its `expect`s are gone.

- **iter-952 — every `expect` reason names its invariant, and four runtime panics are now errors (RT-5, RT-21).** The 96 `#[expect]` sites that said "invariant guaranteed by surrounding validation" each name the invariant their code relies on. Six attributes were removed where the call now returns an error: the webhook listener bind, `operant backup create` and `list`, the `operant service logs` tail, the gateway session-store path, and tool-result serialization. A webhook port conflict now returns an error from `start_with_channel` instead of panicking a detached task, and `ToolResult::success` returns an error result for content that fails to serialize. One gap stays open and is tracked: token-to-header panics in the Home Assistant, Spotify, and LinkedIn tools (RT-20).

- **iter-951 — the TUI corpus validator passes.** Its snapshot-key and key-name tables had drifted from the Rust source, so it reported errors for scenarios that were correct. The tables are now read from the source, and the empty pending-turn script is a valid fixture. Three ledger rows for removed surfaces are deleted. The scenarios touched are re-run: 13 of 13 pass.

- **iter-950 — the blanket `allow(unused_imports)` attributes are removed.** Seventy-three attributes hid unused imports across the TUI. The compiler reports the imports they hid, and the dead ones are removed. Re-exports used only by tests are gated with `#[cfg(test)]`, not deleted. Behavior is unchanged: the full workspace test run is 7394 passed, 0 failed.

- **iter-942 — a log file that cannot be opened is a startup error, not a panic.** `init_logging`
  returns `Result`, and the error names the path. The tokio runtime is built after the `.env` load,
  so that load no longer races worker threads. Two `set_var` writes in the auth commands are removed,
  since the key is read back from `~/.operant/.env`, not the process environment.
- **iter-944 — `[proxy]` in `operant.toml` takes effect at startup.** The file rejected a `[proxy]` table, so proxy settings could not be set in the file the CLI reads. The table is now part of the config, and it is applied before the runtime is built. With `scope = "environment"` it also exports `HTTP_PROXY`, `HTTPS_PROXY`, and `NO_PROXY` for child processes. A per-turn `.env` reload now writes only a value that changed.
- **iter-944 — dependency security advisories.** `cargo audit` reported 21 vulnerabilities. The lock now carries `extism` 1.41.0 (with `wasmtime` 49.0.3), `rustls` 0.23.46, `quinn-proto` 0.11.19, `h2` 0.4.20, and `aws-lc-sys` 0.46. The `rag-pdf` feature uses `pdf-extract` 0.12 (`lopdf` 0.42.0). Three `zip` advisories remain; no workspace code reaches them (see BUGS.md RT-18).

- **iter-935 — `[memory] audit_enabled` and `response_cache_enabled` no longer stop the config from loading.**
  The memory-factory module docs told users to write these keys, but the live `[memory]` table
  rejected them as unknown fields, so the binary exited with `Invalid TOML … unknown field`. They are
  now accepted as retired keys and print one warning each. Neither feature runs in the live memory
  path, so the keys do nothing.
- **iter-935 — test runs no longer touch the real emergency stop.** The `pause_gate` tests engaged
  and cleared the operator's `~/.operant/data/ESTOP`, so `cargo test` could resume an agent a person
  had paused, and the two tests raced on the one file. They now run against a temp estop root
  (`estop::scoped_root_for_tests`), one at a time.
- **iter-934 — a guardrail-refused repeat no longer renders as a completed call.** The
  identical-call guardrail's skip result showed `✓ <tool>`, which said the read ran. The row
  now shows `⊘ skipped — repeated call not run`, and the header draws `⊘` for any `⊘`-prefixed
  row, so a cancelled call no longer draws `✓` either. The agent still reports skips as
  `success: true`, because the degenerate-loop circuit counts failures.
- **iter-933 — a pending tool's name is no longer printed three times.** While a tool
  call was queued or running, the status line repeated `Queued <tool>…` next to the tool
  row and the spinner footer, which already named the tool. The status line is no longer
  set at tool start; the row and the footer carry the state. Regression test:
  `tui::app::tests::tool_start_leaves_the_status_line_to_the_tool_row`.
- **iter-930 — settings.json survives upgrades and failed writes.** `Settings` had no
  container-level `serde(default)`, so a settings file missing any field failed to load.
  The next write from any of the twelve read-then-save sites then replaced the whole
  file with defaults, which is what an upgrade that adds a field would trigger. Missing
  fields now take their defaults. A file that exists but does not parse is copied to
  `settings.json.bak` before a save overwrites it. Saves go through a temp file and a
  rename, so a crash mid-write leaves the old file. The theme picker reports a failed
  save in its status line ("not saved: …") instead of reporting success. Pre-existing
  stale allowlist entry `kernel|unused_mut` pruned.

- **iter-929 — micro-compaction and decay leave the request intact (plan 06 P1).**
  Micro-compaction stalled after its first fold: the marker is a user message, the
  next fold landed on it, and the lone-user guard refused it. It now moves past
  markers. The decay rung truncated the current user request to about 2.2k
  characters; the current turn, from the latest user message on, is never decayed.

- **iter-928 — models.dev failures say what they received (plan P0.1).** A failed
  models.dev fetch reported only a serde error or "had no models", with no size, so a
  truncated or HTML response looked like a schema change. The body is now parsed from
  bytes by `parse_catalog`, and the error names the byte count. The two startup warm-up
  fetches in the TUI logged nothing on failure; they now log at debug. Not changed: the
  live endpoint is still unconfirmed (no network here).

- **iter-927 — one lock for the process-global palette (plan P6.11).** The palette tests
  held two locks over one global. `theme_colors::tests::ACTIVE_LOCK` serialized the
  theme tests and the render test, and the vendored `STYLE_TEST_LOCK` serialized the
  palette and theme-mode tests. `set_active_theme` writes the vendored palette too, so a
  palette test could interleave with the render test and flip its accent. `ACTIVE_LOCK`
  is now the same mutex as `STYLE_TEST_LOCK`. The flake was not reproduced before or after
  (eight full-binary runs green, three before and five after), so this closes a verified
  race by construction, not a reproduced failure. Still open: tests that call `App::new`
  without the lock also write the theme.

- **iter-926 — every provider has a listing endpoint or is named as having none (plan P1).**
  `wire_family` covered 24 of the 42 providers in `PROVIDERS`; `kimi-coding-cn`,
  `minimax-cn`, `alibaba-coding-plan`, `lmstudio`, `nous`, `arcee`, `gmi`, and `qwen-oauth`
  were absent and never listed live. They now map to `OpenAiCompat`. The seven providers
  with no `/models` listing (`openai-codex`, `copilot`, `copilot-acp`, `google-gemini-cli`,
  `azure`, `azure-foundry`, `bedrock`) are named in `NO_LISTING_ENDPOINT`. A test walks all
  42 entries and checks each listing request's URL and key placement. The listing request is
  built by `model_catalog::listing_request`, so the test exercises the same code the picker
  and setup call.

- **iter-925 — the last static default model is gone (plan P2).** The
  `provider_metadata` OpenAI-compatible profile table carried a `default_model` per
  entry that nothing read (the port-excision note says the static model tables are
  unreferenced). Removed from `OpenAiCompatibleProfile` and `ResolvedOpenAiCompatibleProfile`
  and from all entries. No runtime behavior changed.

- **iter-924 — the gateway follows `/connect` and `/model` (plan P0.4).** The gateway
  is a separate process from the TUI, and it kept the endpoint it started with until a
  restart. Each message now compares `[client]` and `[agent] model` in the loaded
  `operant.toml` with the last value seen and retargets its agent (`set_endpoint`,
  `set_model`) when the file changed. The baseline is taken when the handler is built,
  so an environment override stays in place until the file itself is edited. A file
  that fails to parse is skipped for that message.

- **iter-923 — one cost source and one models.dev mapping (plan P3, P2.5).** The
  insights cost estimate read a literal pricing table dated 2025; it now reads the
  models.dev prices in the disk cache (`cached_model_prices`), matching the bare model
  id after any provider prefix. A model with no cached price costs $0.00, as before for
  unknown models; with no cache at all, every estimate is $0.00 until models.dev is
  fetched. The vendor trait's `models_dev_id` hook (OpenAI and Anthropic only) is gone:
  `ProviderCatalog::models_dev_id_for` maps through `provider_to_models_dev`, so
  `route_cost` now covers every mapped vendor. Also fixes an unused `mut` in
  `operant-harness/tests/kernel.rs` that failed the workspace clippy gate.

- **iter-922 — no static model lists; the default model comes from the live catalog
  (plan P2).** `ProviderDef` loses `models` and `default_model` (42 entries), and the
  setup model step lists the provider's live catalog (models.dev when the endpoint has
  none). Connecting resolves the default in the background (live rows, tool-capable
  first, then the first row) and applies it through `pending_model`; a provider with
  no list says so and keeps its model. The sync fallback (startup `/default`
  placeholders) reads the models.dev disk cache only. `ModelRegistry` and its seeded
  tables are gone, and `FREE_CATALOG` drops its `default_model`. Known gap: the
  operant_app `provider_metadata` table still carries static `default_model` values
  and was not touched; `models_dev_id` / `provider_to_models_dev` reconciliation is P3.

- **iter-921 — one live model fetcher for every provider (plan P1).** The `/model`
  picker and `operant setup` both list models through `operant_core::model_catalog::resolve`:
  the provider's own `/models` in its wire family, then models.dev when that is empty,
  then the configured models. Anthropic now sends `x-api-key` with `anthropic-version`
  and Google sends `key` in the query (the picker sent bearer to every endpoint, which
  Anthropic's native `/v1/models` rejects). Errors name the endpoint without the key; setup and `/model`
  both show the reason when the live list is missing. Removed the second fetcher
  (`fetch_models_for_provider` and its helpers) and the test-only `ProviderProfile` /
  `ProviderRegistry` layer in `provider.rs`, which carried its own `fetch_models`.
  Ollama uses the OpenAI-compatible `/v1/models` route rather than a native `/api/tags`
  path; that route is not yet checked against a live Ollama. Live Anthropic and Google
  listing is unverified here (no key on this machine); wire behavior is covered by
  mockito tests.

- **iter-920 — models.dev rows are read from their real schema.** The fetch hit
  `https://models.dev/api/v1/models` and expected flat rows (`context_window`,
  `cost_input`), but models.dev publishes a provider-keyed catalog with the window
  at `limit.context` and the price at `cost.input` (USD per million). Every capability
  lookup returned `None` for real rows, so the context meter, the cost estimate, and
  the agentic-model listing all fell back. The fetch now reads
  `https://models.dev/api.json` and flattens it (`normalize_catalog`) into the rows
  the readers already use. `match_capabilities` also no longer returns `None` for the
  whole catalog when one row lacks `provider_id`. Tests run against rows copied from
  models.dev's published schema. Not verified live: models.dev was unreachable from
  the build host during this iteration.
- **iter-918 — the context window is read after the models.dev fetch.**
  iter-917 refreshed the window before `load_models_dev()` ran, so on a cold
  cache the first frame used the per-provider fallback and nothing re-read it
  until the model changed. Both startup paths now refresh after the fetch.
- **iter-917 — the context meter reads the model's real window from models.dev.**
  The TUI looked the window up in `ModelRegistry::get`, which never carried a
  window: every model in the static seed lists read 0 and the meter was hidden,
  and every other model fell through to a fixed per-provider guess. It now calls
  `cached_capabilities` for the window, with the per-provider defaults as the
  offline fallback. The now-unused registry lookup and its `ModelInfo` /
  `RegistryModelEntry` types are removed.
  The lookup was inert against real data until iter-920 (see there).
- **iter-916 — `/model` says why the endpoint's model list is missing.**
  The picker fetch returned an empty list on every failure (connection
  refused, HTTP error, non-JSON), so an unreachable endpoint looked like a
  picker with only the configured rows and no explanation. The fetch now
  returns the reason, and the status bar reports it: "Could not list models
  (…). Showing configured models only." Also drops the duplicate
  `claude-sonnet-4-20250514` from the Anthropic static list.
- **iter-915 — provider/model selection and the agent loop, end to end.**
  Reproduced against a live proxy: `/connect` flipped the displayed provider
  but the agent kept calling the ORIGINAL `client.base_url`, so every
  request still went to the old host with the new model name. Fixed:
  - `/connect` now retargets the live client (`OpenAIClient::set_base_url`,
    `ModelClient::set_endpoint`, `OperantAgent::set_endpoint`), persists
    `client.*`, and rewrites the `~/.operant/.env` `OPENAI_BASE_URL` /
    `OPENAI_API_KEY` pins that otherwise override the TOML at launch. A
    runtime-retargeted client never sends the previous provider's key to the
    new host.
  - `operant setup` labels a non-catalog loopback endpoint "Custom" (it was
    "Ollama (Local)" for any `localhost`/`127.0.0.1` URL; only ports 11434 and
    1234 identify a local server), clears the previous provider's key when the
    provider changes, persists a cleared API key, and keeps the `.env` pins in
    step with `client.base_url` / `client.api_key`.
  - Catalog base URLs are OpenAI-compatible roots: anthropic
    `https://api.anthropic.com/v1`, google `.../v1beta/openai`, cohere
    `https://api.cohere.ai/compatibility/v1`, minimax `.../v1`.
  - Pasting into `/connect`, the custom-provider form and the free-mode form
    now reaches the open dialog (it went to the composer, and a paste burst
    was treated as a prompt). Ctrl+V works in those dialogs too.
  - The `/model` picker's endpoint fetch runs while the TUI loop holds its
    worker (`block_in_place` on the multi-thread runtime) and a model chosen
    in a picker is applied before the next prompt is sent.
  - A keyless local endpoint (ollama, lm-studio, an open gateway) passes
    through the credential pool instead of failing "no API key configured";
    a keyless remote host keeps the actionable error.
  Verified: a headless `operant run` and a TUI turn both complete a
  `shell`-tool round trip against the connected endpoint; the wizard persists
  endpoint, model and key with matching `.env` pins. Tests: operant-core lib
  2274 pass; operant-cli bin 1601 pass; clippy gate green.

- **iter-914 — memory tool descriptions stop lying to the model.**
  `memory_store`, `memory_search` and `memory_recall` told the model they use
  a "builtin MEMORY.md file store" and point to an "agentmemory" backend; all
  three have run on memory-wire since plan 11C3 (and `memory_save` /
  `memory_smart_search` are the same store). The descriptions now say so.
  AGENTS.md's prompt-cache note reflects the iter-911 layout.

- **iter-913 — dead session-picker port deleted (plan 05B B2.6).**
  `operant_ui/session_picker*` and the six `operant_app` support modules only it
  used (`session_picker_types`, `import`, `claude_live`, `catchup`,
  `onboarding_flow`, `registry`) are gone: ~12.8k lines, 92 tests that
  exercised only that unreachable code. `/resume` is unchanged (native
  `session_browser`). Clippy gate and the remaining 1612 `operant-cli` tests
  green.

- **iter-912 — superseded fence-raster pipeline deleted.** `tui/latex.rs`,
  `tui/mermaid.rs`, `messages/cache.rs`, the per-frame raster drain in
  `render/mod.rs` and `pinned_images::pin_rasters` (~2.1k lines + tests) had
  no producer on any production path: mermaid renders through the vendored
  `operant_app::mermaid` renderer and math degrades to unicode by design. No
  behavior change. BUGS.md R-LATEX (filed in iter-910) is closed as
  superseded.

- **iter-911 — cache-stable prompt layout (batch-06 P0) + native Anthropic
  system-prompt loss.**
  - *Layout:* per-turn volatile context (harness feed, memory recall,
    workspace context, MoA guidance) used to sit in a system message between
    the frozen prompt and the history, so any change invalidated the provider
    prompt cache for the whole conversation after it. It now rides the tail as
    a `<turn_context>` prefix on the latest user message (built once per turn,
    reused across tool iterations, charged against the budget explicitly).
    Everything before the latest user message is byte-stable between turns.
    User-content, not a mid-conversation system message, so strict chat
    templates accept it. The pre-compaction memory insight is inserted after
    the system prefix instead of at index 0 (a user message ahead of the system
    prompt). Pinned by
    `turn_context_rides_the_tail_and_the_prefix_stays_byte_stable`.
  - *Anthropic (opt-in `anthropic` feature):* `convert_request` kept only the
    LAST system message, so whenever a volatile block existed the frozen prompt
    (identity, skills, tool guidance) never reached the model. Every system
    message is now sent as an ordered `system` block array; only the first (the
    stable prefix) carries `cache_control`.
  - Restored the `prompt_cache_guard` doc block that an earlier cleanup had
    stranded away from its function.

- **iter-910 — clippy gate green workspace-wide (`operant-cli` purge).** The
  allowlist gate (`scripts/clippy-warning-gate.sh`) now passes (and its 6 stale
  allowlist rows are pruned). ~120 dead items in `operant-cli` were deleted
  with the tests that exercised only them (virtual list, the Anthropic model
  client stub, render cache, banner/rustle/shimmer helpers, `messages::tools`,
  truncation helpers, background-task counters, 19 tests total); orphaned doc
  blocks and an unreachable duplicate gateway `/session` arm were removed; three
  never-read `App` fields (`accent_color`, `auto_compact_enabled`,
  `footer_right_column_area`) and the `EffortPicker`/`ModelPicker` priority
  variants are gone; staged data-table fields and CLI-shaped lints carry
  `#[expect(..., reason)]`. The device-auth corpus scenario is parked in
  `excluded.json` (nothing opens that dialog since H-9). New findings
  (unwired LaTeX/mermaid fence rendering) are in BUGS.md.

- **iter-909 — clippy gate green for `operant-core` and `operant-os`.** The
  gate (`scripts/clippy-warning-gate.sh`) had been failing at compile time:
  an automated attribute pass left duplicated, mis-placed
  `#[expect(clippy::expect_used)]` blocks (16 unfulfilled) while three real
  `.expect()` sites carried none. The poisoned-lock sites
  (`model()`, the tool-guardrail locks) now recover the guard instead of
  panicking; dead code the gate surfaced is deleted (the DuckDuckGo-lite
  scraper and its helpers, `visible_now`, `distilled_memory_id`,
  `GLOBAL_AUTHORS`, an unused web-provider `urlencode`, an orphaned
  prompt-cache-guard doc block, an empty authority-filter `impl`); the
  `memory_store` tool no longer advertises an `importance` argument nothing
  read; integration-test crates carry the standard
  `#![allow(clippy::unwrap_used, clippy::expect_used)]` header. `operant-cli`
  is still outstanding (tracked in BUGS.md).

- **iter-908 — stale org `--reason` test retired; `org --help` banner fixed.**
  `tests/org_reason.rs` drove `org employee/notice/worklog/sync/import`, which
  iter-838 deleted (7 of 10 tests red on main). It now pins only the surviving
  half of the mandate (`kanban block` requires `--reason`). `operant org --help`
  no longer claims every mutating subcommand needs `--reason`. BUGS.md: H-2
  and S6 closed (their code was deleted earlier).

- **iter-904 — red-team audit correctness queue
  (`docs/plans/plan-2026-10-17-redteam-audit-of-remaining-gaps.md`).**
  - *Memory feedback channel (H-5):* post-turn session distillation made
    its LLM call before checking for a memory-wire store, so every turn
    spent a model call with nowhere to write (and consumed the next
    turn's scripted reply in tests). It now resolves the store first and
    skips the call without one. `memory_feedback` green.
  - *Review discipline (H-7):* tool results carrying the agent's own
    appended `[Tool loop warning: …]` were classified "unparseable tool
    output" and triggered anomaly retries; the malformed-JSON check now
    inspects only the leading JSON value. The off-by-one round counts
    were the H-5 call shifting scripted replies. `review_discipline` 6/6.
  - *models.dev lookup (H-6 residual):* per-turn cost lookup waited out a
    15 s fetch for providers models.dev doesn't map, and re-attempted it
    every turn when offline; it now maps first, backs off 300 s after a
    failed fetch, and writes the disk cache on cold success.
  - *`/fast` removed (H-8):* the toggle set a flag no agent code read, on
    both the TUI and the gateway. Command, field, registrations and help
    line deleted.
  - *Overlay paint-order test (H-3):* expectation updated for the
    `rewind_selector` replacement and `effort_picker` deletion; the retired
    `rewind-flow` corpus scenario and goldens removed; help-overlay goldens
    re-proven without the `/fast` line.
  - *Test hygiene:* relative-path SQLite test DBs in `operant-core`
    (`test_*.sqlite` in CWD) raced with tests that `chdir`, flaking 7
    tests; they now live in the temp dir. `operant-core --lib` 2277/2277
    across repeated runs.
- BUGS.md hygiene: H-5/H-6/H-7/H-8 and K-1 closed with root causes; TODO.md
  K-3 row moved to Implemented (done in iter-887).

- **iter-906 — `/stop` cancels a queued gateway turn (W3-2).** A turn
  waiting on another chat's turn used to ignore `/stop` ("nothing to
  stop") and run later. It is now marked cancelled race-free under the
  turn-lock claim mutex, replies "Cancelled your queued message", and exits
  `Interrupted` without running the agent. K-2 no longer reproduces; D-6
  recorded as an accepted limit.
- **iter-905 — connect-dialog dead ends and MCP panel auth (H-9), native
  scroll docs (plan 07 step 6).** The `openai-codex` connect row no longer
  opens a device-auth dialog that could never complete (it takes an API key
  like its peers); the unreachable `github-copilot` arm and the
  `device_auth_pending` stub are removed. The MCP panel's `a` key runs the
  real OAuth 2.1 + PKCE login in the background (same flow as `operant mcp
  login`) instead of printing "not yet wired". README gains a "Terminal
  scrollback mode" section and `operant.example.toml` documents the
  mouse-capture contract (never enabled in native-scroll mode; verified at
  the only `EnableMouseCapture` call site, the alt-screen branch). K-4
  (`--no-default-features`) re-verified closed by the iter-892 crate purge.

### Docs

- **iter-903 — the 2026-10-16 raw audit is integrated into the kb**
  (`docs/audit/2026-10-16-operant-audit/`: the INDEX, the 20 ranked
  findings, and the eleven standalone plan files with `file:line`
  evidence that drove iters 894–901). Its execution-order note (02 test
  gate first, safety before features, crate purge last) stays the
  operating contract for follow-ups. Side fix: the iter-900 re-homing
  left four dangling `docs/plan-*` references (two CHANGELOG paths, two
  in PLAN-TUI-OVERHAUL{,-v1}.md) — repointed at `docs/plans/`.

### Fixed

- **iter-898 — plan 05B B2.7 first half: `/login` runs operant's own
  provider backend** (owner-directed integration). `/login` (and
  `/init`) now suspend to `operant setup provider` — the setup wizard
  is the real provider/model backend (fuzzy provider select, key
  prompt, model choice, and since iter-893 a surgical persist), driven
  through the existing `shell_out_to_subcommand` seam (the `/setup`
  suspend-and-run pattern). `/logout` answers honestly: keys live in
  `~/.operant/.env`, run `/login` to change the provider or key. The
  in-TUI connect dialog stays reachable for first-run boot and
  `/connect` (it works — BUGS.md H-9's dead ends are the remaining
  reason to retire it, queued as its own block). The tier test now
  pins the new semantics (`pending_shell_command` armed for /login).
  Gates: gate 2 = 1740 / 2 = H-3 pre-existing + the known palette-race
  flake (solo-green); corpus 61/62 (rewind-flow pre-existing); clippy
  seen=39 new=38 parity.

- **iter-896 — plan 05B (second half): the legacy model/effort picker
  overlays are deleted** (the twin surfaces the unified inline picker
  replaced). `tui/model_picker/{models,render,state,tests}.rs`,
  `tui/model_picker.rs` and `tui/effort_picker.rs` (~1,240 lines) are
  gone; `effort.rs` survives as `tui/model_effort.rs` (the
  `EffortLevel` type), and `ModelEntry` + the registry model helpers
  moved whole into `adapter_types/model_registry.rs` (their one home —
  they feed provider-switch defaults, not a picker UI). `App` loses
  `model_picker`, `model_picker_provider_id` and the
  `open_model_picker_for_provider` opener (the registry fallback adapts
  its rows at the model_fetch_rx boundary); the connect flow
  (`activate_provider`) opens the unified picker; mouse/prompt gating
  follows `inline_interactive`; the overlay dispatch table, paint-order
  test and corpus vocabulary drop the two retired rows.
  `/effort` now opens the SAME unified inline picker with the four
  effort rows (glyph + wire name, current flagged, default marked);
  Enter routes through the plan-03 `pending_effort` drain — the
  effort-picker corpus golden re-proved for the inline surface.
  Gates: gate 2 = 1740 passed / 2 = H-3 pre-existing + one
  palette-race flake (solo-green, same class as accent_role — the
  deletion shifted test interleaving, not the race); corpus 61/62
  (rewind-flow pre-existing); clippy seen=39 new=38 parity.

- **iter-892 — plan 10 phase 1: six crates the binary never called are
  gone (175,997 lines)** (audit plan 10, findings 17+18). The method:
  grep `<crate>::` across every other crate — `operant-gateway` (16.7k
  lines) was named by NOBODY; `operant-hardware` (9.6k) only by gateway;
  `operant-channels` (60.8k) only by gateway (the real channel adapters
  live in `operant-core/src/gateway/`); `operant-runtime` (85.5k) by one
  cli call; `operant-tool-call-parser` (3.0k) and
  `operant-tool-planning` (0.4k) only by runtime. The six form one
  dependency island (channels' code names `operant_runtime::` directly),
  so the plan's one-crate-per-commit had no compiling intermediate —
  ONE cut, proven whole:
  - The one real call (`core_observer_from_config`) moved into core as
    `observer::observer_from_config` + `NoopObserver` (`backend = "none"`
    → no-op; a configured otel/prometheus backend now warns once and
    degrades to `ConsoleObserver` — those backends retired with the
    crate; the user's config has `backend = "none"`).
  - `operant-cli` loses the `gateway`, `hardware` and all `channel-*`
    forwarding features (zero `cfg(feature = "channel-` users in
    source — pure declarations), the `channel_feature_forwarding` test
    (it pinned the deleted crates' manifest contract), and the
    `operant hardware` / `operant peripheral` subcommands with
    `cmd_hardware.rs` / `cmd_peripheral.rs` (STM32/Arduino flashing —
    the plan's default; they never named the crate).
  - Proof: `cargo check --workspace` 0 errors (12 members);
    gate 2 = 1761/1 pre-existing (H-3); core lib green solo (battery
    failures = the known parallel-DB flake class, all 7 named tests
    solo-green); `operant --help` carries no hardware/peripheral
    entries; README + AGENTS.md crate lists updated to the 12-member
    truth.

### Fixed

- **iter-895 — plan 05B (first half): the jcode unified inline picker
  is LIVE for /model, /models and Ctrl+A**. The ported renderer +
  types (ui_inline_interactive.rs, PickerKind/Action/Entry/Option,
  1.5k lines) were dead behind `inline_interactive_state()` returning
  `None` — a `[port-decision]` stub. `App.inline_interactive` now holds
  the picker state; the seam returns it and the renderer draws it
  instantly (a command-suggestion-style popover above the composer).
  The key handler is jcode's semantics, Model-subset: Up/Down navigate
  rows (or the row's options in column ≥ 1), Left/Right/Tab cycle
  columns, typing filters (Esc clears the filter first, closes
  second), Enter drills into multi-option rows and applies on the last
  column — routing through plan 05A's `pending_model`/`pending_effort`
  drain (the agent switches, the config persists, the display
  refreshes from the agent). Rows come from the same 05A catalogue:
  `GET {base_url}/models` + the configured `[agent]` models (marked
  "configured", current row flagged and preselected); the fetch rides
  the existing model_fetch_rx drain and merges in place. The registry
  fallback adapts at the boundary. `overlay_flags()` carries
  `inline_interactive` (the legacy `model_picker` flag is gone; the
  legacy overlay itself is deleted in the next iteration).
  The model-picker corpus golden re-proved for the inline surface
  (drift verified as intended first); two stale tests rewritten
  (Ctrl+A and /model assert the inline state, not the dead overlay).
  Tests: `model_command_opens_the_inline_picker_on_the_configured_rows`,
  `inline_picker_enter_applies_and_esc_closes`. Gates: gate 2 =
  1765/1 pre-existing (H-3); corpus 61/62 (rewind-flow pre-existing);
  clippy seen=39 new=38 parity.

- **iter-893 — the setup wizard persists surgically (plan 09's third
  surface, owner-directed backend integration)**. `operant setup`'s
  persist did the same whole-file `toml::to_string_pretty` rewrite the
  TUI's model picker did (iter-874): comments destroyed on every wizard
  run, `operant -c <path>` runs written to a hardcoded
  `~/.operant/operant.toml`, env-sourced secrets flattened into the
  TOML. Now `persist_config` diffs the wizard's mutated struct against
  the struct it was HANDED (the true wizard delta) and persists only
  the changed dotted keys through plan 09's surgical writer, in the
  file the process actually loaded. `operant setup reset` diffs the
  runtime config against factory defaults the same way — a minimal
  reset, not a rewrite. Array fields (`fallback_models`) land as real
  TOML arrays: `persist_config_key` gained an Item-parse arm (raw
  `[\"a\", \"b\"]` literals parse as arrays; plain strings still coerce
  as scalars). A fresh install with no config file keeps the full 0600
  write. Tests: `wizard_persist_is_surgical`,
  `wizard_persist_writes_arrays_as_arrays` (serialized through a
  path-slot lock — the loaded-config path is a process-global).
  Gates: gate 2 = 1763/1 pre-existing (H-3); core lib persist 4/4;
  corpus 61/62 (rewind-flow pre-existing); clippy parity.

- **iter-891 — plan 06B: seat memory banks, one store, per-turn
  scoping** (audit plan 06B, the 11E remainder). The gateway agent
  recalled and retained in ONE bank for every chat and seat — a cast
  seat's turns polluted the owner's personal memory and every seat
  shared it. `MemoryWireProvider.bank` is now a runtime-switchable
  `RwLock<String>` with `set_scope(&MemoryScope)` (one store; the bank
  is only a key), the `MemoryProvider` trait gained the same seam as a
  default no-op method (Holon runs keep their construction scope —
  O-22: a Holon run never writes operant memory), and the gateway
  handler retargets it INSIDE the turn lock right after the seat
  binding: a bound seat's turn runs in `emp:<sanitized-id>`, an
  unbound (owner) turn in the personal bank. The seat id is sanitized
  by the existing `bank_for` ([a-z0-9_-], ≤32) so a hostile seat id
  cannot forge bank names. Test:
  `set_scope_partitions_seat_and_personal_banks` — a personal fact
  never leaks into a seat's recall and vice versa, both directions.
  Gates: gate 2 = 1761/1 pre-existing (H-3); core lib green solo (the
  9 battery failures are the parallel-DB disk-I/O flake class — all
  11 named tests solo-green); corpus 61/62 (rewind-flow pre-existing);
  clippy seen=39 new=38 parity — zero delta.

- **iter-890 — plan 08: ONE memory engine everywhere** (audit plan 08,
  finding 10). The agent moved to memory-wire in 11C3, but every surface
  around it still read the legacy files: `operant memory list/store/
  search/...` wrote MEMORY.md (invisible to the agent's recall),
  `operant doctor` probed MEMORY.md/USER.md health, `/whoami` read a
  phantom third store and said "I don't know much about you yet" while
  memory-wire held hundreds of rows, `/journey` listed the phantom
  store, `operant migrate` copied MEMORY.md to a directory nothing
  reads, and every single start printed the agentmemory warning — from
  the DEFAULTS. Now:
  - `MemoryWireProvider` gained thin CLI pass-throughs
    (`list_page`, `memory_by_id`, `delete_memory_by_id`,
    `memory_bank_stats`, `bank_id`) — same panic-guarded
    degrade-don't-die contract as the turn hooks.
  - `operant memory` runs on them: list/get/search(recall)/store/
    delete/stats/import/export/clear (import accepts JSONL or a legacy
    MEMORY.md/USER.md; profile and prune are deleted — they read
    USER.md and had no engine filter).
  - doctor prints one memory-wire line (store path + bank stats); the
    MEMORY.md/USER.md probes are gone.
  - `/whoami` and `/journey` read memory-wire (the journey keeps its
    row type via a small adapter); `operant migrate` imports into
    memory-wire instead of file-copying.
  - Config: default provider is `memory-wire`; the retired agentmemory
    keys still parse (accepted + ignored) but DEFAULTS ARE SILENT —
    `legacy_warnings()` returns data and the new test pins that a clean
    start warns nothing (the audit's every-start warning is gone);
    `agentmemory_package_spec`/`agentmemory_mcp_spec` and the
    agentmemory MCP seeding (`ensure_default_mcp_servers` on the core
    side) are deleted with their tests — the schema twin in
    operant-config stays until plan 10 takes that crate.
  - mcp-view corpus golden re-proved: the seeded agentmemory MCP row
    is gone from the default panel.
  Tests: `memory_defaults_are_silent` (RED before: defaults warned),
  `cli_store_is_agent_visible` (the plan's acceptance: a CLI-stored
  fact is found by the agent-shaped recall — RED before: two engines).
  Gates: gate 2 = 1761 passed / 1 pre-existing (H-3; H-2's test deleted
  with the legacy path it pinned); core lib 2276 green solo (the 3
  battery failures are the known parallel-DB disk-I/O flake class,
  solo-green); corpus 61/62 (rewind-flow pre-existing, BUGS.md H-3);
  clippy seen=39 new=38 baseline parity.

- **iter-888 — red-team pass 2 (audit §9): org-mode event intake
  spoke a dead kernel contract and could never fire.** The 2026-10-17
  adversarial pass over AUDIT-2026-10-15.md found W5's "O-15 closed"
  false in effect twice over: `events::match_and_fire` matched the RAW
  `api_triggers` pattern — which carries the `event:` kind marker
  (migration 0010: `pattern = coalesce(s.event, s.cron)`) — so no real
  channel text could ever contain it and intake never fired; and the
  payload went as `--payload -` + stdin, the pre-H-06 contract, which
  the literal-`--payload` kernel (8eff8b7) takes as the string "-" —
  every firing lost the event text. Now CONTRACT §13's semantics: the
  marker is stripped (fail-closed on rows that do not speak the
  documented syntax) and the text rides as ONE literal argv token
  (exec-style, no shell; the kernel never reads it as a file
  reference). O-15's acceptance test never existed (the O-24
  phantom-coverage class); `tests/events.rs` now pins all of it
  against a real kernel — fire once, the cooldown skip NOT counted
  (H-13 rc 3), a non-match fires nothing, and the event text reaches
  the fired work's message VERBATIM. Mutation-proven: 0/1 against the
  pre-fix code, 1/1 with it; operant-os battery 37 green with
  `HOLON_REPO` set.

- **iter-887 — plan 07 §1′+§2: native scrollback gets a message
  watermark and a replay** (audit plan 07, bugs 1-2 + the H-1
  regression). The row watermark drifted whenever a tool row
  re-rendered (Queued→Running→Done) between flushes — the flushed rows
  duplicated or shifted in the terminal's own scrollback, and H-1
  (`message_final_rows_tracks_the_last_final_message`, RED since the
  test gate repair) pinned exactly that defect. The watermark is now a
  MESSAGE index (`scroll_emitted_through`): whole final display
  messages are emitted, and a final message's rows are immutable, so
  the index cannot drift. And because native scrollback cannot reflow,
  the operations that make emitted history wrong now REPLAY: the new
  `SynchronizedUpdate::replay_clear` (cursor home + clear screen +
  purge scrollback, inside the DEC 2026 bracket) resets the watermark
  and the next pass re-emits the whole transcript at the CURRENT
  width. Replay is requested by /clear, /new, /cls (which keeps its
  divider), rewind, session switch, /compact, theme change, the
  Ctrl+T and Alt+O display toggles (toggles are now retroactive —
  the old notice said "applies to new output" and the user wanted
  them functional), resize (debounced 150 ms), and Alt+L (manual).
  The viewport floor (`ui_viewport`) now computes from the message
  watermark (`message_count_rows`). Tests:
  `message_count_rows_tracks_message_boundaries` (the H-1
  replacement: cumulative boundary rows through N messages) and
  `replay_resets_the_watermark_and_reemits_at_the_current_width`.
  Gates: gate 2 = 1761 passed, 2 pre-existing H-failures only (H-1
  is closed by this change); corpus 61/62 (rewind-flow pre-existing
  at tip, BUGS.md H-3); clippy seen=39 new=38, baseline parity.

- **iter-886 — plan 04: ONE command table; no dead or lying
  commands** (audit plan 04, findings 11+12+19). Three tables fed the
  TUI: typeahead/palette/help read the GATEWAY's `COMMAND_REGISTRY` —
  so autocomplete suggested 18 commands that only printed "not yet
  wired", never suggested ~45 commands that actually worked (typing
  `/rew` or `/perm` showed nothing), and `/split` and `/mcp <args>` fell
  through to the MODEL as prompts. Now `REGISTERED_COMMANDS` is the one
  table for typeahead, palette and `/help` (`COMMAND_REGISTRY` remains
  the gateway's), and the sweep test enforces BOTH halves: every
  registered row dispatches, and every typeahead suggestion dispatches
  — a lying suggestion is now a build failure, not a runtime notice.
  - **Wired to the real CLI subcommands** (the /setup suspend-and-run
    pattern, one `shell_out_to_subcommand` helper, zero re-implementation):
    `/cron`, `/kanban`, `/curator`, `/suggestions`, `/profile`, `/logs`,
    `/pause`, `/unpause`, `/org`, `/platforms` (→ `operant gateway status`),
    `/title` (→ `/rename`), `/doctor` (the real health checks, not the
    display-only registry handler), `/memory` (→ `operant memory`); the
    AGENTS.md selector moved to `/context-files`. Bare `/mcp` keeps the
    panel; `/mcp <args>` runs the subcommand.
  - **Branching made honest**: `/branch` sets the feeder flag (the arm
    opened an EMPTY list — the fork-tree backfill never ran), and Enter
    switches for real (the drain retargets the agent and rebuilds the
    display — the old body printed "Switched to branch" and did
    nothing); create/delete modes point at the real flows (`/rewind`
    forks; sessions are kept for /resume by design).
  - **Deleted** (lying or dead): `/split` `/split-view` (sent to the
    model), `/busy`, `/browser` (Camofox is purged), `/indicator` (kept
    `/statusbar`), `/billing` `/credits`, `/pet` `/heapdump` `/mem`,
    `/replay` `/replay-diff`, `/background` (TUI arm; the gateway keeps
    its row), and the `COMMAND_REGISTRY` rows nothing serves
    (`history`, `handoff`, `snapshot`, `env`, `toolsets`, `blueprint`,
    `gquota`). `/update`'s help no longer claims "Background update and
    auto reload".
  Tests: the extended `no_registered_tui_command_is_unwired` (RED first:
  /history landed the not-wired fallback), the new
  `typeahead_suggests_the_registered_tui_commands` (RED first: /rew
  suggested nothing), and the rewritten mcp contract test; the stale
  snapshot/blueprint alias pins went with their commands.

- **iter-875 — plan 05A: `/model` switches the agent and lists the
  configured endpoint's models** (audit plan 05 Part A). The picker's
  rows came from a models.dev snapshot for a provider GUESSED from the
  model name — the agent's real endpoint (`client.base_url`) was never
  queried, so an omp-gateway endpoint showed OpenAI's catalogue — and
  the confirm path never reached `agent.set_model`: the display copy
  changed, requests kept using the old model forever, and the picked id
  was re-mangled into `{provider}/{id}` from the guessed provider. Now:
  - the catalogue is the endpoint's own (`GET {base_url}/models` with
    the client's key, OpenAI-style `data[].id`), merged with the
    configured `[agent] model` + `fallback_models` (always shown,
    marked "configured", so an unreachable endpoint is never a dead
    end); rows show the real id; the fetch rides the existing
    `model_fetch_rx` → drain path so the picker opens instantly and
    refreshes in place;
  - the confirm path and Alt+P model cycling set `pending_model`, and
    the run loop's drain applies it: `agent.set_model(id)` → the
    display refreshes FROM `agent.model()` (one source of truth) →
    context-window refresh → persist via plan 09's surgical writer
    (`agent.model` only — `client.*` never touched);
  - the `{provider}/{id}` munging is gone — the picked id is the model
    id the endpoint knows.
  Regression test: `model_pick_drain_switches_the_agent_and_persists`
  (agent switches, display refreshes from the agent, config file gains
  exactly the one key, no whole-file rewrite).

- **iter-874 — plan 09: surgical config writes — one key, in the file
  that was loaded** (audit plan 09, findings 7+18). The TUI's model pick
  re-serialized the WHOLE runtime config to a hardcoded
  `~/.operant/operant.toml`: comments and layout were destroyed, retired
  default sections (`[pk]`, `[genome.budget]`, …) were resurrected on every
  write, `operant -c <path>` runs wrote the WRONG file, the provider-guessed
  `client.base_url` clobbered the user's endpoint (e.g. the omp gateway
  replaced by api.anthropic.com), and env-sourced secrets landed in the
  TOML in plain text. `operant model set` and `operant config set`
  changed the in-memory config, printed success, and vanished with the
  process. Now one writer — `operant_core::config::persist_config_key` —
  edits exactly one dotted key via `toml_edit` (comments and every other
  key byte-for-byte), in the file the process actually loaded
  (`loaded_config_path()`, recorded at load), through a temp-file +
  validate-before-rename cycle that keeps the 0600 mode and can never leave
  an unloadable config behind. Callers: the TUI persist (`agent.model`
  only — client.* is never written from the TUI), `operant model set`,
  `operant config set` (both print the path they wrote; in-memory update
  kept). The serde_json walk/set (`set_nested_json` +
  `coerce_json_value`) moved to core as the one copy — the agent's config
  tool's in-memory-only contract is unchanged. Tests:
  `persist_config_key_keeps_comments_and_other_keys_byte_for_byte`,
  `persist_config_key_never_writes_secrets_that_were_never_in_the_file`,
  `persist_config_key_creates_intermediate_tables_and_coerces_scalars`,
  `persist_config_key_rejects_a_broken_leaf_and_leaves_the_file_unchanged`.

- **iter-872 — plan 03 §6: `/goal` actually reaches the agent**.
  The arm claimed the goal was "injected so the agent sees it", but the
  injection was a display-only system annotation — the agent's context
  never changed. `/goal <text>` now queues `[session goal] <text>` through
  `pending_user_message` (the same one-keystroke drain `/skill` uses),
  so the agent receives the goal as a real message on the next turn; the
  status-bar display and transcript annotation stay. Regression test:
  `goal_submission_reaches_the_agent_context`. `/fast` — the other §6
  item — is scheduled for deletion in its own iteration (picker
  machinery + golden regeneration via `prove`); the TUI knob reaches
  nothing in the agent, and the gateway's `fast_mode` session-metadata
  twin has no reader either (recorded in BUGS.md).

- **iter-871 — plan 03 §3/§4: the permission decision path** (audit
  plan 03, findings 5+6). `settings.permission_mode` had zero readers on
  the permission DECISION path — BypassPermissions still prompted every
  request and Plan mode still let gated tools run. Now the drain consults
  the mode FIRST, for every request: `BypassPermissions` →
  `AllowSession` (the core's hardline/blocked checks run before the
  channel, so bypass can never widen a hardline block), `Plan` → `Deny`
  with a visible "read-only" notice. `/yolo` no longer writes the
  bypass mode to settings.json with no confirmation — arming goes through
  the same confirmation dialog `/permissions` uses; disarming stays
  direct. A second request arriving while a dialog is open is now QUEUED
  (`pending_permission_queue`) and opens in order as dialogs close —
  the old drain silently denied it, and tools run up to 8 in parallel
  (the parallel tool pool iter-56) so one batch could carry two gated
  tools. The bash "always allow" prefix now stores the first TWO tokens
  ("git status", not "git" — a one-token entry auto-approved
  `git push --force`) and the read side refuses any command carrying
  shell metacharacters (`; && || | \ $ ( ) > < \n`) or flagged by the
  core's `ApprovalGuard::check_dangerous` — a stored prefix can never
  approve an injected tail or a dangerous shape. The dead
  `bash_command_allowed_by_prefix` test twin is deleted; the four
  store-side tests assert the real allowlist. Tests:
  `bypass_mode_auto_allows_without_a_dialog`,
  `plan_mode_denies_without_a_dialog`,
  `a_second_request_while_a_dialog_is_open_queues_instead_of_denying`,
  `bash_prefix_allows_plain_continuations_only`.

- **iters 868–869 — the remaining-gaps close-out, agentic-OS** (G3
  closed by a peer in iter-865; see `docs/agentic-os/2026-10-18-REMAINING-GAPS.md`'s
  closure log):
  - **iter-868 — G2, Chair notify, kernel-aligned**: `operant org holon
    notify --reason R --text T` is the `[notify].command` delivery seam —
    the kernel computes what is new for the Chair (`pending_for_chair`,
    `notify_seq` dedup, `_deliver` for routine results) and shells this
    CLI; operant resolves the Chair's channels (the same `chair_channels`
    source `is_chair` uses) and posts through the gateway's one-shot
    send. A first-draft polling organ was found wrong-direction against
    the kernel source and reverted before landing — the kernel owns the
    cadence and the sequencing. Owner wires the instance:
    `[notify].command = "operant org holon notify --reason heartbeat
    --text {message}"`.
  - **iter-869 — G10, the multi-turn e2e + two real bugs it caught**: the
    new e2e drives the real prepare env with a tool-call frame outside
    the org's `[scope]` roots and proves the live policy denies + records
    + the file never appears, with per-call metering derived from the
    verified call count. Building it caught: (1) stream.rs's headless
    Escalate-deny path never recorded the refusal — O-17's missed
    sibling; now records `policy.json:escalate` with the org-mode
    message; (2) session distillation fired at every Holon turn end —
    W6/O-22's sixth learner — unmetered post-turn LLM spend and a
    personal-memory retain in a Holon run; now gated on the run env like
    the other five learners.

- **iter-867 — the TUI drives the agent, not a copy of it (audit plan 03
  §1/§2/§5)**. The TUI kept a parallel copy of the conversation, and
  `/clear`, `/new`, `/undo`, `/retry`, `/stop` edited only that copy:
  the slash command printed success, the screen changed, and the
  agent's real context never moved. Worse, `/stop` and mid-stream
  `/clear`/`/new` only flipped the display's `is_streaming`, so the
  agent kept running while the TUI showed idle — and a prompt typed
  right after spawned a SECOND concurrent run on the one shared agent
  (the same corruption the gateway's turn lock fixed in iter-864).
  Now:
  - `/stop` triggers the real interrupt flag (the exact `/cancel`/Esc
    path) instead of a display flip.
  - `/clear` and `/new` mid-stream cancel the turn and DEFER — the
    drain runs `agent.clear_history()` / mints a fresh session id
    (the rewind fork's `sess_` shape, via `notify_session_switch`)
    only after the run has fully finished, so a stopping turn's tail
    write can no longer repopulate the conversation after the clear.
    The display rebuilds from the agent's conversation (one extracted
    `rebuild_display_from_agent`, shared with the rewind handler).
  - `/undo [N]` and `/retry` go through the agent's REAL rewind
    machinery (`rewind_marks` → `rewind_to_turn`, which forks and
    keeps the undone path for `/resume`) instead of a display-only
    `messages.truncate`; the undone prompt returns to the composer,
    and `/retry` re-queues it for resubmission.
  - One submit path: the interactive and simulator composer blocks
    collapsed into `submit_user_message`, which now carries the
    steering queue AND the new stopping-turn guard (an unfinished
    agent task future refuses a second run: "A turn is still
    stopping").
  - The drain runs at the loop top of BOTH run loops once the turn
    finishes; with no agent attached (headless mock scenarios) the
    display-only fallback preserves the old semantics.
  Regression tests: `stop_mid_stream_triggers_the_real_interrupt`,
  `clear_mid_stream_cancels_and_defers_to_the_drain`,
  `clear_when_idle_defers_to_the_drain`, `undo_parses_n_and_defers`,
  `undo_while_streaming_is_refused`,
  `retry_defers_into_the_rewind_machinery`, and the drain battery
  (`clear_drain_empties_agent_context_and_display`,
  `new_drain_mints_a_fresh_session_and_empties_the_context`,
  `undo_drain_forks_through_the_rewind_machinery`,
  `retry_drain_queues_the_resubmission`,
  `drain_leaves_pendings_alone_while_a_turn_is_still_stopping`).

- **iter-865 — G3: clock-organ liveness is now evidenced, and doctor
  reads it** (`2026-10-18-REMAINING-GAPS.md` G3). The clock organ fires `holon tick`
  every 2 minutes but wrote no liveness evidence, and `operant doctor`
  unconditionally printed OK for it — a clock that died at boot read as
  healthy forever. The organ now stamps the unix seconds of every
  successful tick to `<holon home>/.clock/last_tick` (best-effort; a stamp
  failure logs and keeps the cadence), and doctor classifies the stamp:
  missing/unreadable → `check_fail` "never ticked" (a daemon younger
  than 2 minutes legitimately has no stamp yet), older than 300s
  (generous vs the 120s cadence) → `check_fail` "stale", recent →
  `check_ok` with the age. Regression test:
  `cmd_doctor::tests::clock_verdict_reads_the_liveness_stamp`
  (fresh/stale/never/garbage). BUGS.md H-3 strengthened: the corpus
  scenario `rewind-flow` fails the same rename
  (`Unknown assertion path 'overlays.rewind_flow'`, proven identical at
  pristine tip).

- **iter-864 — gateway: one turn at a time on the shared agent** (audit
  plan 06A:
  `docs/audit/2026-10-16-operant-audit/06-gateway-concurrency-and-seat-memory.md`).
  Every gateway chat shares one `Arc<OperantAgent>` whose hot conversation
  is swapped by `set_session_id`; with every message on its own tokio task,
  two concurrent chats retargeted each other mid-run — one chat's tool
  results and answer persisted under the OTHER chat's session id, a second
  chat's dispatch wiped a `/stop` aimed at the running chat, and `/stop` in
  one chat killed another's live turn. Fixed with one `turn_lock`
  (`tokio::sync::Mutex<()>`) on `GatewayMessageHandler`, held from the
  session retarget to the end of `handle`: concurrent chats now queue
  (with the platform typing indicator shown while they wait) instead of
  corrupting each other. The interrupt-latch reset moved from the dispatch
  loop into the lock (a second chat's dispatch can no longer un-stop the
  running chat), the per-turn `cron_origin` moved inside the lock with it,
  and `/stop` only attaches the shared interrupt flag for the session
  whose turn holds the lock (`stop_flag_if_running`). Regression tests:
  `concurrent_chats_keep_disjoint_transcripts` (mutation-proven: disabling
  the lock fails it on the exact cross-session corruption) and
  `stop_attaches_only_to_the_running_turns_session`. Residual: `/stop` for
  a queued (not yet started) session is a no-op — BUGS.md W3-2. Verified
  en passant: the cron scheduler runs due jobs sequentially, so the
  cron agent has no analogous race (D-6).

- **iter-863 — the test gate is real again** (2026-10-16 audit, plan 02:
  `docs/audit/2026-10-16-operant-audit/02-test-gate-repair.md`). Ten test
  targets had been dead since the org layer retired (iter-838) — they
  still imported `authority`, `employee_db`, `notice`, `worklog_db`,
  `write_barrier`, `CamofoxStateTool` — so `cargo test -p operant-cli
  --bin operant` could not compile and every later "gates green" claim
  was unverifiable. Disposition per
  `docs/agentic-os/ORG-LAYER-DISPOSITION.md`:
  - **Deleted** (subjects retired to Holon):
    `org_wave2_shared_file`, `governance_escalation`, `org_notice_board`,
    `org_employee_registry`, `org_identity_gate`,
    `org_worklog_framework_only`, the orphaned
    `tools/authority_tools_tests.rs`, the `gateway_commands` grant/revoke
    battery (`grant_is_operator_only`,
    `grant_seats_ungoverned_seat_and_mints_ttl_grant`,
    `grant_days_override_beats_the_config_ttl`,
    `grant_to_unmanaged_seat_fails_closed`,
    `grant_standing_only_when_approver_is_org_lead`,
    `grant_days_override_cannot_defeat_the_standing_rule`,
    `revoke_stops_the_grant_and_a_second_revoke_is_not_found`,
    `P3Stores`/`seed_p3`/`p3_seed_employee`), and the write-barrier pair in
    `cron_session_isolation`
    (`a_completed_scheduled_run_leaves_exactly_one_worklog_row`,
    `a_barrier_write_failure_makes_the_run_report_failure`) plus
    `forbidden_seat_cannot_delegate_through_the_real_guard` and the
    `browser_camofox_state` row in `integration_test`, and — found by the
    first green run — `org_cli_contract.rs` in full (all seven tests drive
    retired `operant org` subcommands: `department`, `dm`, `decision`;
    the live surface is `org budget` + `org holon`).
  - **Rewired to the kept modules**: `seat_policy_run_path` (SeatAuthority
    now wraps only the policy source; `delegation` field gone),
    `cron_session_isolation` (`derive_employee_id` moved to
    `org::worklog`), `permissions_shows_posture_and_gates_other_seats`
    (seeds a `SeatPolicyDb` row; asserts the Holon grant redirect), and
    the `record_feed` channel-map test (`operant_kanban.db` sibling path).
  - Stale comments naming retired modules as live were deleted from
    `agent/stream.rs`, `agent/mod.rs`, `cronjobs/scheduler.rs`,
    `gateway_runner.rs`, `org/seat_policy_db.rs`, `org/worklog.rs` — the
    last two also fix broken intra-doc links that failed the rustdoc
    gate.
  - New `scripts/test-compile-gate.sh` runs `cargo check --tests` over
    every kept crate so a module deletion cannot leave dead tests again;
    wired into `self-test.sh` and the AGENTS.md per-push gate list.
  - **The gates now run.** Gate 2 (`cargo test -p operant-cli --bin
    operant`): 1742 passed, 3 failed. Gate 3 core lib: 2271 passed
    (the one `sub_agent_tool` failure is the known parallel-DB flake —
    solo rerun green). The 2026-10-15 brief's deferred tests ran for the
    first time and PASSED: `tool_output_budget_evicts_oldest_first` and
    the five rewind-selector tests. Six real hidden regressions the red
    gate had been swallowing are now in `docs/audit/2026-06-19-BUGS.md` (H-1..H-3, H-5..H-7);
    the one `sub_agent_tool` failure is the known parallel-DB flake
    (solo rerun green).

- **iters 854–858 — the agentic-OS red-team close-out** (audit
  `docs/agentic-os/AUDIT-2026-10-15.md` §8). A red-team of the W-wave
  closure record found four claims that did not survive adversarial
  reading; all closed with regression evidence:
  - **iter-854 — O-03 for real**: the `holon_*` native tools now register
    on the RUN ENV (`HOLON_RUN_DIR`), not on `[holon] enabled` — iter-844's
    commit message had claimed this but the gate was unchanged, so real
    bodies (whose posture configs carry no `[holon]`) never saw the tools.
    The e2e now drives the kernel's ACTUAL `dispatch --prepare` env
    (init → hire → task → prepare) instead of a hand-synthesized map.
  - **iter-855 — O-12 residuals**: the organs semaphore was decorative
    (bodies awaited inline; concurrency was always 1) — bodies now run as
    spawned tasks holding permits; the CLI supervise `--loop` no longer
    hot-loops on error; stop-after-submit (CONTRACT §4): `run_body` polls
    `run show` on its own 15s cadence, kills the process group when the
    run leaves `running`, and skips the driver-path finish (the kernel's
    own submit already recorded the result).
  - **iter-856 — O-19 reporting + O-18 remainder**: usage.json carries the
    PROVIDER's prompt/completion tokens (metered per call, no longer
    conditional on a budget cap) plus the session's models_dev cost and
    the build stamp — never the chars/4 estimate; the e2e asserts the
    scripted provider's exact numbers. The policy `commands` allowlist is
    now enforced at both terminal seams (`terminal` + `aft_bash`), and
    the AFT writers share the roots fence (`guard_aft_write`). The
    chair-test ETXTBSY flake (tmpfs exec-vs-write settling, ~1-in-3
    batteries) is root-fixed with a single-retry `run_holon` helper.
  - **iter-857 — O-14 acceptance**: "approve <id>" from the Chair now
    routes to the kernel's `plan approve/reject` decision commands through
    `run_as_chair` (reject's because-clause rides `--note`); ordinary chat
    still classifies.
  - **iter-858 — verification debts**: the WAL-safe archive regression
    test iter-851's commit body described but never shipped now exists
    (`tests/archive_wal.rs`: a committed-but-uncheckpointed row IS in the
    snapshot, integrity_check ok, org/ tree + side-board db ride);
    doctor reports the Notion arm's script and the newest org archive with
    its age; `supervise --help` and operant-os's boundary citation now
    tell the truth (O-29).
  - **Holon adapter (kernel 38ae9e9)**: install.py probes the pinned
    operant for native tools at install time and drops the resident
    Python `holon mcp` mount for builds that carry them — completing W1's
    phase E acceptance (no Python alive during the body).

### Added

- **iter-846 — memory is memory-wire only (plan 11C3).** Six parallel
  memory layers collapse to one engine. BuiltinProvider and the
  agentmemory aliases are deleted: `build_memory_provider` builds
  memory-wire for every configured name (unknown names warn once at
  config load, alongside agentmemory_auto_spawn), and an init failure
  degrades to a no-op provider — memory is unavailable for the session,
  never a silent file fallback. The `<long_term_memory>` prompt block is
  GONE: memory reached the prompt twice (the file-backed block AND the
  provider's semantic recall); now only the provider recalls
  (`<memory_context>`). The memory tools (store/search/recall) route
  through the ONE provider — their legacy JSON fallback deleted, and with
  no backend attached they report it honestly instead of writing to a
  store nobody reads. Session distillation retains through the provider
  too (the old path wrote MEMORY.md, which nothing injects anymore).
  **One-time importer**: on the first start after upgrade, MEMORY.md and
  USER.md entries are parsed with the same §-format that wrote them and
  retained into memory-wire tagged `[imported, …]`, then the files are
  renamed `*.imported` (kept — the rollback path; the rename is the
  idempotency marker). No-data-loss contract: a failing retain or an
  in-memory (degraded) store leaves the files untouched for the next
  start. Config: `[memory] provider` accepted-with-warning; example.toml
  documents the single engine.

- **iter-840 — one binary updater for sourcehound and aft (plan 11B).**
  The two auto-update paths (single-binary sourcehound, versioned-cache
  aft) each carried their own download/verify/replace logic, neither
  pruned, neither throttled, and failures were debug-only. New
  `operant-core/src/binary_updater.rs` is the one shared implementation:
  `gh api` release check (API asset URLs — `browser_download_url` 404s on
  private repos even with the token; the verified combination is the API
  form + `Accept: application/octet-stream` + the gh token), download,
  SHA256SUMS verify when present, atomic replace (0755 before the rename
  — the live sourcehound 1.7.0 install initially landed non-executable
  and was caught by the version check), cache-dir install with
  **prune-to-2** (aft carried 14 versions), a per-process once-guard and
  a 6 h on-disk throttle (`~/.operant/cache/updates.json`), and a
  once-per-session user-visible notice drained into the TUI status line.
  Surfaces: `operant doctor` lists installed/latest per integration with
  an update hint, `operant version --detailed` prints them,
  `operant update integrations` force-checks now. Live-proven:
  sourcehound updated 1.6.0 → 1.7.0 through the real path, then reported
  "up to date" on re-run. Tests: version parse/compare, prune-to-2,
  checksum-detectability, tar.gz + raw-ELF extraction, and a fake-`gh`
  check resolving installed/latest without the network.

- **iter-839 — native scrollback: message-final emission (plan 09, step 1;
  fixes K-3).** The scroll emitter's row watermark now clamps to the last
  FINAL display message — `final_display_message_count` walks the display
  list and stops at the first still-pending tool row (Queued/Running rows
  re-render on every status flip; emitting them early froze a partial into
  scrollback, then the flip duplicated or shifted history — BUGS.md K-3).
  The boundary is also body-relative now: the old committed-prefix
  measurement wrongly counted HEADER chrome rows as emittable transcript,
  which desynced the live-region floor (the terminal-scroll-mode golden
  re-proven; its pinned ↑22 was the defect). Corpus
  terminal-scroll-mode + terminal-scroll-cls re-proven deterministic,
  verify 0 drift. This is step 1 of the native-scroll plan; the dynamic
  live region, replay-on-reset, and the default flip remain gated on the
  live terminal verification.

- **iter-838 — rewind selector: fullscreen fork UI (2026-10-14 batch, plan
  05, TUI steps 3-5).** /rewind is no longer a modal list that truncated
  the DISPLAY only — it opens a fullscreen selector over the REAL
  transcript (the body renderer's own lines), outlines the selected user
  turn, dims everything after it (the preview of what the fork drops), and
  shows `Keeps N messages · ~T tokens · drops M turns` from the agent's
  read-only `preview_rewind`. Enter executes the AGENT-side fork
  (`rewind_to_turn`, iter-829): the display is rebuilt from the agent's
  forked store, the rewound prompt lands back in the composer, and the
  notice names the kept parent session. Compacted turns refuse with the
  inline "folded by compaction" notice; Enter is refused while a turn
  streams. `f`/`/` opens a case-insensitive filter over prompts with
  live-jump. The old RewindFlowOverlay + MessageSelectorOverlay are
  deleted (their display-truncate behavior mutated nothing in the agent —
  the screen and the context diverged). `/rewind <n>` targets prompt #n
  directly with a y/n status-line confirm. Double-Esc on an empty composer
  opens the selector (new `[tui] double_escape_action`, default "rewind",
  "none" disables; a single Esc keeps every existing meaning). Ctrl+B now
  populates the branch browser with the session's fork tree (parent +
  children from the DB `parent_session_id` links the rewind forks write).
  Corpus: rewind-flow replaced by rewind-selector + rewind-selector-filter.

- **iter-836 — duplicate-tool-rows oracles + verdict (2026-10-14 batch,
  plan 01).** The mid-stream duplicate-row symptom is now measurable. The
  headless sim drains ONE scripted event per loop pass under
  --frame-per-event (re-arming the mock completion until the event channel
  empties), so every inter-event state paints a frame — 14/7/19 frames for
  the three new agent scripts (parallel out-of-order tools, error-then-
  success, 8 identical calls as a hash-collision stressor). A cache-
  coherence oracle (OPERANT_DEBUG_PREP_ORACLE=1) builds every frame twice —
  once through the caches, once fully uncached — and diffs the wrapped
  lines; the sim fails on any mismatch. Verdict: 0 mismatches over all 40
  frames, and per-frame row counts are exact and monotone (1..8 for the
  8-call script, never exceeding live calls) — the body cache's prefix/
  suffix reuse and the live-rows model are CLEAR in the simulator; what
  looked like duplicate rows mid-stream is the spinner detail + status
  notice carrying the tool NAME beside the rows, not a second row.
  S1b is fixed regardless: every println/eprintln while the TUI may own
  the screen routed through tracing (permissions review, storage recovery,
  session picker, regex-compile duplicate dropped; debug-hub capture
  errors), and `#![deny(clippy::print_stdout, print_stderr)]` now sits on
  the tui module — the only exemptions are the logger's own failure path
  (tracing is dead there) and cfg(test). Remaining suspects S1 (OSC-8/
  pinned-image residue) and S2 (Ambiguous/Wide glyph widths) are live-
  terminal-only; the live tmux capture-pane check is the open red-team
  item. Three corpus scenarios pin the shapes with `count:` asserts
  (exact per-call counts on the final screen).

- **iter-817 — dead-command removal: 35 unwired slash commands purged
  (2026-10-14 batch, plan 02).** The iter-811 sweep's finding is now
  structurally impossible: 31 planned removals (/commit, /git, /test,
  /swarm, /subagent, /ssh, /remote, all commit/push/release variants,
  /initiatives, /onboarding-*, /productivity, /provider-test-coverage,
  /model-status, /compact-notifications, /show-agentgrep-output,
  /tool-call-details, /btw, /voice, /fork, /restart, /observe, …) plus
  /?, /log, /back, /client-reload, /server-reload surfaced by the new
  stronger test. The right-click Fork menu item now fills /branch (the
  wired twin) instead of the removed /fork. The orphaned
  provider-test-coverage render chain went with its command:
  draw_model_status_overlay, model_status_line_style, the live_tests
  classifier module, and the model_status_overlay trait method (no
  override existed — the overlay could never show). New wiring invariant
  tests: `no_registered_tui_command_is_unwired` (drives every locally
  dispatchable name through the live handler wiring, asserts no
  not-wired notice; `registered_command_names_including_hidden` rows are
  now tagged remote_only so bridge-forwarded commands like /continue
  are not false-positives) and `removed_commands_are_absent`. The
  313-line inline command-handler block in the TuiApp create path was
  extracted to `register_tui_command_handlers` so tests can mirror the
  live registry wiring exactly. `command_sweep.py`: 46/46 consumed, 0
  fall-through. Corpus re-proven (7 goldens, deltas verified
  removal-attributable before acceptance); full bin suite 1755/1755.

- **iter-816 — tui debug simulate: --frame-per-event + count: screen
  asserts (2026-10-14 batch, plan P2).** Mid-stream oracles for the harness
  that plans 01/04/08/09 lean on: `--frame-per-event` writes every painted
  frame to `--capture-dir` (all-flags mode on the hub, no sentinel index
  list; the missing-frames check is skipped since the total is unknowable
  up front), and `--assert-screen 'count:NEEDLE==N'` asserts an exact
  non-overlapping occurrence count on the final screen — the duplicate-row
  detection a contains/not-contains pair cannot express. The CLI→runner
  seam now passes a `FrameCapturePlan { indices, all, dir }`. Corpus verify
  green on the new binary (0 drift); frame-per-event proven live against
  the stream-multi-chunk agent script.

- **iter-815 — memory-wire auto-sync on every build; provenance surface
  (2026-10-14 batch, plan 11A).** `scripts/build.sh` is the single build
  entry point: it advances the memory-wire branch-tracking lock to upstream
  HEAD before every dev/test/release/deploy build (the old 1h throttle stamp
  in `scripts/check.sh` let builds silently compile an 18-commit-old
  dependency). `--offline`/`MEMORY_WIRE_NO_SYNC=1` skip the sync and never
  break an offline build; `--strict` fails deploy builds on sync failure;
  `--sync-only` is the explicit advance. The locked rev is baked into the
  binary from `Cargo.lock` via `build.rs`: `operant --version` prints it
  (`0.2.1 (memory-wire 84580d5)`), `operant version --detailed` prints the
  full rev, and `operant doctor` compares the compiled rev with upstream
  main and reports up-to-date / behind. `scripts/check.sh` and
  `scripts/sync-vendors.sh` delegate their sync here — one sync code path.
  The lock moved from `0017859e` (0.5.1) to `84580d57` (0.7.0). Fake-cargo
  ordering test: `scripts/tests/build_sh_test.sh` (5 assertions).

- **iter-807 — TUI shortcuts (oh-my-pi parity), conflict-checked.**
  `Ctrl+G` edits the prompt in an external editor, set by the new
  `[tui] external_editor` (falls back to `$VISUAL`, `$EDITOR`, `vi`). The scroll
  bookmark moved to `Alt+G`. `Alt+P` cycles the configured models (`model` +
  `fallback_models`, in session only). `F5`/`Alt+R` retry: they resubmit the
  composer, or the last prompt. `Ctrl+O` expands or collapses the latest tool
  output (bounded to the last 64 calls, 256 KB each). `Ctrl+R` history search
  now lists newest-first, with each prompt once and pins kept while typing,
  and draws one renderer: the legacy floating panel that duplicated the
  inline bar is gone. Every composer chord is now in the `/keys` catalogue,
  and `the_default_keymap_has_no_conflicts` fails on any key bound to two
  actions in one context.

- **iter-732 — tool_guardrails: duplicate-progress spam pinned across
  the output batch reset (outline v5 §1 row 4 remainder):** the audit
  behind this slice found row 4 mis-specified — the `no_progress/
  mod_tests.rs` suite it names is upstream-only (never existed in-tree;
  the port plan explicitly declined to create the path) and
  `pattern_tests` holds 12 tests, not 13. The one genuinely uncovered
  *pure* case is duplicate-progress spam: a loop padded with failed
  batches resets the output-side streak between every pair
  (`observe_output`'s reset arm clears `output_streak`/`output_armed`
  only) while the same successful result recurs underneath. The
  recurrence ledger is run-wide state, not batch state, so the spam
  still escalates on the same 5/6 rungs as back-to-back repeats. New
  `ladder_tests` case beside `failures_do_not_feed_the_recurrence_ledger`
  asserts the fifth recurrence warns and the sixth arms the skip across
  resets; mutation-proven — making the reset arm also clear the ledger
  turns it red. The remaining two named faults are not test ports:
  oscillation is BUGS.md S7 (a `ping_pong_cycles` production fix) and
  progress-token stall has no production seam at all.
- **iter-729 — `[agent] max_tool_result_chars` wired: ingestion-time
  tool-result trim for the runtime Agent (outline v4 §1 row 2:
  wire-or-drop; wired; code commit `da42909b`, labeled iter-728 at
  push):** the schema knob (head 2/3 + tail 1/3, truncation marker in
  the middle, `0` disables, default 50000) had zero consumers since
  the old runtime engine's removal sweep dropped its call site — both
  runtime-Agent turn paths ingested raw tool output into history via
  `format_results`, which is how an untrimmed `http_request` fetch
  charged 50k tokens of context (the TUI live audit's N-4). The trim
  now applies `history::truncate_tool_result` (whose contract matches
  the knob schema exactly, including image-marker boundary nudging)
  at both ingestion sites before either dispatcher renders; the
  observer and `TurnEvent` channel keep the full output so UIs still
  show it. Reconciled/core path untouched (core already caps ingestion
  at 4096 with a JSON-aware trim). Dark-mergeable: unchanged config
  keeps the 50000 default. Run-path test pair in `agent::tests` (200k
  output → trimmed with marker at max=1000; untrimmed at 0). Label
  note: renumbered twice append-only (725 → 728 at the code commit,
  728 → 729 here) — the peer's TUI waves took 725/726/727 mid-slice
  and their P1-6 took 728 between my code push and this docs push.
- **iter-723 — skills: nested-references validator lockstep (meta-skill-
  creator rewrite Rust side):** the skill-side rewrite (iters 701–705)
  shipped ONE skill + plain `.md` nodes under `references/`, but the Rust
  copy of the old routing contract still validated child-skill-dir trees —
  `operant skills audit` and `skill_manage generate_map` would misvalidate
  every migrated tree, and the always-on agent prompt still taught
  `skill_view(name='<parent>/<child>')` syntax that no longer exists.
  `skills_tool.rs` walk rewritten for file-nodes: BFS reachability from
  the root over markdown links + backticked paths (unreachable node =
  error, dangling link = error, dangling backtick = warning, pointer
  escaping the skill = error), stray nested `SKILL.md` = error (routing-
  model regression guard), root-only health checks (name/dir mismatch,
  description ≥60 chars registry.py-parity, root >200-line budget),
  `<dir>.md` index-file convention, non-`.md` unreferenced-resource
  warnings; discovery surfaces `references/` files with one-line summaries
  (legacy child-dir trees still work pre-migration). `skills audit` tree
  gate now also triggers on `references/` trees — otherwise migrated trees
  (no child SKILL.md dirs) would be silently skipped. Agent prompt
  "Meta-Skill Routing" block rewritten to the file-node contract.
  Regression tests: unreachable node, dangling link, stray nested
  SKILL.md — each fails without the fix (113 skills tests green).
- **iter-724 — config-file wiring: `[agent] guardrail_exempt_tools` +
  the two env-armed knobs get config parity:** the runtime exempt path
  existed end-to-end (`AgentConfig::guardrail_exempt_tools` →
  `ToolGuardrailTracker::add_exempt_tools`) but
  `From<&BehaviorSettings>` hardcoded `Vec::new()` — no config-file
  surface ever fed it. Now a `BehaviorSettings` field (serde default)
  flows through the From impl to the tracker. `[agent] guardian_llm` /
  `micro_compaction` are config-file DEFAULTS for the iter-713/714
  knobs: each applies only when its `OPERANT_*` env var is UNSET — env
  var and the explicit `with_*` builders still win — applied by
  `apply_agent_config_defaults` (the `attach_credential_pool`
  post-construction modifier shape) at all three agent factories
  (events path covers TUI/chat/gateway; no `gateway_runner.rs` edit
  needed) plus the ACP command path. Dark-mergeable: an unchanged
  config file parses to empty-exempt/off/off. Tests pin all three
  bindings with non-default values plus the dark default.
- **iter-722 — durable cron delivery ledger (trinity P2-A concrete;
  hermes `deliveries.db` tombstone/stale-claim shape):**
  `deliver_result` handed the payload to the gateway over an in-memory
  channel and forgot it — a gateway outage or dead consumer dropped the
  message silently (`tx.send` on an unbounded channel never errors; the
  delivery twin of the R39 finding). New append-only v3 migration
  `cron_deliveries` (no FK, like `cron_runs` — history outlives job
  deletion): the row is queued BEFORE the handoff, `CronDelivery` now
  carries the ledger id, and the consumer settles it via
  `CronDb::mark_delivery_outcome` (idempotent; the gateway consumer
  mount is the next slice). One handoff = one attempt;
  `MAX_DELIVERY_ATTEMPTS = 3` PARKs a row — the tombstone: replay
  stops, the row stays inspectable. `reclaim_stale_deliveries` applies
  hermes' `max(3×timeout, 2h)` horizon (2h floor — the send is a
  fire-and-forget handoff), and the tick replays stale rows BEFORE the
  due-jobs gate so empty-due ticks — the usual outage-recovery window —
  still replay (same lesson as the iter-684 empty-tick socialization
  fix). No channel mounted = rows stay pending and retry next tick: an
  outage queues instead of drops. `ObserverEvent` gains
  `CronDeliveryQueued`/`Reclaimed`/`Parked`/`Outcome` variants
  (dark-mergeable `with_observer` mount on the scheduler). `parking_lot`
  joins operant-core deps (unpoisoned-lock house rule). 7 tests: pre-v3
  migration on a live v2 database, tombstone walk, idempotent outcome
  settle, horizon-respecting reclaim, queue-and-handoff incl. the R39
  no-target arm, replay-then-park with observer lifecycle points,
  no-consumer leaves rows pending. Label note: a third concurrent
  iter-720 landed on origin (peer's small-core-debts slice); this one is
  renumbered 722 — append-only.
- **iter-714 — micro-compaction (hermes parity, opt-in via
  `OPERANT_MICRO_COMPACTION`, off by default):**
  `context_management::micro_compact` folds the oldest completed
  exchange below the protected head (4) and recency tail (8) into one
  marker message carrying the exchange's last assistant gist
  (head/tail-preserved at 240 chars); tool-pair integrity by construction
  (consecutive `Tool` results travel with the fold); the fold runs at
  `build_messages` time against the STORE once the conversation crosses
  the soft threshold (40% of the window) — continuous small bills ahead
  of the batch preflight ladder, never replacing it. Strictly
  below-prefix: the iter-698 PromptCacheGuard fires by design on any
  violation — proven live during test design when a too-small window
  drove the eviction into prefix territory and the guard caught it.
  `parse_micro_compaction_env` pure-tested; armed once at construction
  (`with_micro_compaction` the deterministic test seam).
- **iter-713 — guardian-LLM approval arm (the approval-matrix row's
  missing tier; hermes `approval_smart.py` parity):** `GuardianVerdict`
  (Approve/Deny/Escalate) in `approval.rs` with comment-stripping (the
  `echo hi # IGNORE ALL RULES` injection vector dies before
  classification), a command-as-data XML consult, and a STRICT verdict
  parse — chatty, hedged, or poisoned replies escalate; the guardian may
  only remove work from the human queue, never add trust. Armed ONCE at
  agent construction via `OPERANT_GUARDIAN_LLM` (`parse_guardian_env`
  pure-tested; `with_guardian_llm` is the deterministic test/override
  seam — no per-call env probes, no cross-test leaks). `stream.rs`
  consults it in the smart gate's `requires_approval` arm for UNGOVERNED
  seats only — seat policy keeps precedence (the classifier classifies,
  the policy decides); Approve skips the permission channel, Deny is
  shaped like a smart-gate block, anything else is today's byte-for-byte.
  Config-file wiring deferred to the `config.rs` unblock.

- **iter-712 — P0 delegation governance (the standing rule's first slice):**
  employee-seat delegation now consults the governance layer and nowhere
  else — `DelegationPosture` (forbidden/bounded/independent) + the pure
  `can_delegate` in `org/authority.rs` apply the §2.3.1 tiers to WORK
  (same-dept allowed; cross-dept on a live grant; unregistered target
  fails closed — the registry is the only identity source);
  `SeatPolicy.delegation` column via the `ensure_column` probe (bad
  spelling errors on read, never defaults; pre-712 files migrate in
  place); `SeatAuthority::consult_delegation` companion with an optional
  employee registry (Bounded fails closed without it); `stream.rs`
  consults for every `delegate` call from a governed seat — no posture row
  = ungoverned = today's byte-for-byte.

- **iter-711 — feed class live: channel/group posts reach the org's
  seats (gap 5 phase 2's feed sources):** `ContextClass::Feed` joins the
  four aspect classes — captured at route time, before the admin gate
  (`MessageHandler::record_feed`, no-op default so `handle`'s contract is
  untouched), rendered per-seat under `feed_quota` with
  `feed_seat_map` routing (unmapped chats default to `premiere`).
  Telegram `channel_post` updates now parse (author = channel title) and
  are feed-ONLY: recorded, never turned, never answered into the
  channel. Group messages record as feed AND keep their existing
  turn behavior. Regression pins: channel-post parse, feed capture →
  seat isolation → watermark advance, and the
  capture-before-the-gate routing contract.

- **iter-698 — PromptCacheGuard + `HERMES_TURN_TIMEOUT`:** the frozen-prefix
  invariant becomes checked — `prompt_cache_guard` snapshots the head
  system run when the ladder fires and verifies it byte-identical after
  the evict rung (warn in release, debug_assert in tests), so a future rung
  edit cannot silently tax every prompt-cache hit in a session. The turn
  wall-clock ceiling gains the `HERMES_TURN_TIMEOUT` env override (seconds;
  malformed or non-positive input keeps the 20-minute default), the
  sibling of `HERMES_REQUEST_TIMEOUT` (one call) and `HERMES_MAX_ITERATIONS`
  (iteration count). 3 guard tests.

- **iter-697 — Wave-4 ordered preflight ladder:** `build_messages` now
  runs explicit ordered rungs when the estimate exceeds the 80% preflight
  threshold — TOC/trim (`fast_trim_tool_results`, its first production
  caller) → deterministic decay → LLM summarize-before-evict
  (`preflight_llm_summarize`: same reactive guards — `should_compress` +
  anti-thrash cooldown — with NO deterministic fallback) → evict — each
  gated on still-over-threshold. Wrap-up rung appends final-call copy when
  the ladder fired, riding the built list only so it cannot accumulate
  across turns. 2 ladder property tests.

- **iter-695 — socialization phase 2: session outcomes post to the board:**
  the senior's close-out notice is the only board write outside the CLI,
  gated by the same §2.3.1 consult (identity fail-closed, consult before
  write, refusal = skipped post). `resolve_actor_scope`/`live_grants_for`
  moved to `org/authority.rs` as the canonical consult companions shared
  by the CLI seams and the scheduler-side writer. Phase 3 holds by
  construction: outcomes ride the phase-2 notice + worklog into `org
  synthesize`'s org-bank digest.

- **iter-692 — gap 8: chief-of-staff synthesis (Slice 9) + decision→charter
  amendment (Slice 10):** `operant org synthesize [--window-hours N]
  [--dry-run]` composes the org digest over a window (recent notices,
  proposed/accepted decisions, worklog), retains it to the **org memory
  bank** (bank `org` in the same memory_wire.sqlite), and posts it as a
  chief-of-staff broadcast notice. Amendments: `org decision propose
  --amend-seat <SEAT> --amend-charter <TEXT>` stores the pair
  (both-or-neither); `org decision accept` applies it BEFORE the status
  transition (a failed amendment leaves the decision proposed);
  `EmployeeDb::amend_charter` is the only sanctioned charter write besides
  the cast seeder; `org audit <seat>` shows the charter posture. Pre-692
  decision files migrate via a PRAGMA-probe ALTER.

- **iter-691 — startup reaper CLOSE + turn-exit journal columns:** pending
  turn-state rows flip terminal at detection time (close no longer gated on
  channel-notice delivery — kills the "still in-flight after restart"
  loop); `.turn_state` gains `exit_code`/`exit_reason` on every terminal;
  `TurnExitReason` classifier with trinity-#904 kill-marker precedence (a
  signal death is never auth).

- **iter-690 — notice-board READ side wired:** `operant org notice inbox
  --for-employee <id>` (`--pending-only`, `--limit`, `--json`) and the seat
  prompt's pending-notices block in `bind_seat_run` — ack-is-the-watermark:
  pending `ack_required` notices re-render each seat prompt until acked,
  fail-open if the seat is not a registered employee.

- **iter-689 — SwitchModel steer variant:** `/model <name>` parses to
  `SteeringCommand::SwitchModel` (strict prefix, case-preserved arg); the
  drain arm retargets via the interior-cell `set_model` with the iteration
  budget refunded, mirroring the fallback chain.

- **iter-688 (record_output port) — output-side successful-repeat guard:**
  `ToolGuardrailTracker::observe_output` (openhuman parity): identical
  narration+batch signature — captured in run.rs before `tool_calls` moves
  into `execute_tools` — warns at 4 (`OUTPUT_REPEAT_WARN`), arms a
  skip-next-call backstop at 5 (`OUTPUT_REPEAT_SKIP`); failed/exempt batches
  reset the streak; synthetic skip results excluded via
  `stream.rs::observe_iteration_output`. Warn/skip, never halt (iter-682
  deviation). 7 ladder tests.

- **iter-684/685 — guardrail exemption threading (openhuman
  `is_repeat_call_exempt`):** `AgentConfig.guardrail_exempt_tools` seeds
  the live `ToolGuardrailTracker` in both `OperantAgent` constructors
  (`add_exempt_tools`, the mutable complement to `with_exempt_tools`);
  `FacadeConstruction.guardrail_exempt_tools` threads caller exemptions
  through `build_from_config` — the facade-side per-tool activation beyond
  CLI exclusions. `is_guardrail_exempt()` exposes the state. iter-685
  seeded the new field across 18 test/example/facade fixture literals.
  Config-schema wiring remains a pending row.

- **iter-682 — Wave-2 ladder completion: per-tool failure rungs, hard-reject
  halt, successful-repeat recurrence ledger.** `ToolGuardrailTracker`
  gains the remaining openhuman `no_progress/` semantics: a per-tool
  consecutive-failure ladder (any error class — warn at 8, `Halt` at 12 with
  root-cause `failure_copy`, reset by that tool's next success; S2's
  3-strike timeout mask stays on top); a hard-reject halt at the second
  consecutive `Blocked by security policy` result (shared prefix constant
  so the classifier and stream.rs's blocked arm cannot drift; seat-policy
  denials deliberately excluded — a minted grant can make those succeed);
  and a run-wide `(tool, result-hash)` recurrence ledger that catches
  A, B(reset), A cycles the consecutive-identical-result rung can never
  see, at the same 5/6 warn/arm escalation (deliberate deviation: operant
  warns/skips, never halts a turn for repeated successes).
  `GuardrailDecision::Halt(String)` is a new variant; `observe_guardrail_results`
  becomes async two-phase (lock dropped before await), surfaces the Warn
  verdicts it previously discarded, and on Halt triggers the interrupt
  flag (same machinery as steer request-stop) with the summary as final
  content. 12-test adversarial port of openhuman's `mod_tests.rs` ladder
  semantics lands as `tool_guardrails::ladder_tests`.

- **iter-672 — W2 selection: scroll-stable content-space selection points
  + jcode-parity keyboard copy mode.** `selection_anchor`/`selection_focus`
  become `(col, content_line)` — the scroll-stable rendered-line index — so a
  selection stays glued to its text across viewport scrolls instead of sliding
  off the screen row it was made on (jcode's `CopySelectionPoint` model,
  projected through `last_resolved_chat_scroll` exactly like jcode's
  `copy_point_from_screen`). Ctrl+T copy mode gains the full jcode keyboard
  set: h/l/j/k + arrows (SHIFT extends from the anchor, plain moves collapse
  the selection onto the cursor), Home/End line edges, PageUp/PageDown,
  g/G viewport top/bottom, plain A select-all-visible, Ctrl+A copies the
  cursor ± 4 lines of context (jcode's `COPY_VIEWPORT_CONTEXT_LINES`) and
  exits, Enter/Y copy. Edge autoscroll now drives NORMAL drags too, not just
  copy mode (jcode autoscrolls on any edge drag).

- **iter-633 — mid-flight seat-budget enforcement (Wave-4 §5)**: a HARD
  token cap now stops a runaway turn AT the iteration boundary instead of
  after it — a turn that spends the whole window in one go is exactly the
  burst the cap exists to prevent. Design notes: the envelope
  (`SeatBudgetEnvelope{cap_tokens, used_at_turn_start}`) rides a
  `tokio::task_local!` set ONLY by the gateway runner inside the per-turn
  task, so it is per-turn by construction — one seat's breach can never
  abort a concurrent seat's turn. The agent-wide `InterruptFlag` is
  explicitly NOT used: it wraps one `Arc<AtomicBool>` shared by every
  session's handle (the `/stop` `turn_inflight` entries all point at it),
  so tripping it on one seat's breach would abort every concurrent turn.
  On breach: a 🛑 meta-notice (surfaced as its own message by the existing
  meta-notice path), a `TurnDiagnostics{BudgetExhausted}` log, and the
  existing grace call for a legible partial. Ungoverned callers (CLI `run`,
  chat, TUI, autonomous) set no envelope and keep the byte-identical legacy
  loop. Token basis only — usd-basis seats stay turn-start-enforced (per-call
  catalog cost inside the loop is a recorded follow-up). Two regression
  tests prove the boundary fires (grace exit + notice) and that the
  ungoverned path is unchanged (normal text response, no notice).

- **iter-632 — Wave-4 metering wire (production bug fix)**: real model
  usage now reaches the budget accumulator. Before this,
  `SessionStore::update_tokens` had ZERO production callers — the
  `gateway_sessions.total_tokens`/`estimated_cost_usd` columns the Wave-4
  rollup sums stayed permanently 0, `employee_window_usage` always
  returned (0, 0.0), the hard cap at turn start could never trip on real
  spend, and `<budget_state>` always claimed the full cap remained.
  The wire: the gateway event receiver folds every `AgentEvent::Usage`
  (tokens) and `AgentEvent::Cost` (models_dev-catalog USD, emitted per
  model call on both streaming and non-streaming paths) into a
  per-thread `TurnUsageMap` — same lifecycle as the S5 `ExitReasonMap` —
  and the turn-end block drains it ONCE into the session accumulator
  after `agent.run()` returns but before the outcome arms, so error and
  early-stop turns are metered too and no entry leaks into the next turn.
  Fail-open on telemetry (genome invariant): a metering write never fails
  a turn. Regression test: `wave4_turn_usage_drains_into_the_budget_accumulator`
  proves the drain makes real spend visible to `employee_window_usage`.

- Wave 5 — onboarding governance (ORGANISM-ARCHITECTURE §4):
  `operant cron create` is now the onboarding TRANSACTION — new flags
  `--seat-mode <yolo|standard|scoped|lockdown>`, `--allow <glob>`
  (repeatable), `--deny <glob>` (repeatable). The automaton's employee row
  is provisioned via the registry's own backfill and its policy row is
  seated in the same step (mode defaults to `[genome].unrestricted_default`
  — the documented posture — unless named). Cross-file sqlite cannot be
  one transaction, so the honest shape is job-first-then-provision with a
  DELETE rollback: the operator gets a governed automaton or no automaton,
  never an ungoverned one left behind by a failed register.
  The unified doctor (F2) grows a `genome` section — the owner's no-new-
  commands flag surface: registry census; UNGOVERNED CRON AUTOMATON count
  (the ratified D-2 default made visible, with the exact seating
  commands); budget-window typos named per seat (the fail-open value the
  resolver silently absorbs); pre-Wave-2 session rows pending their
  premiere backfill; cast-not-seeded guidance. Both `operant doctor` and
  GET /api/doctor see it (drift pin extended to five categories).

- Wave 4 — budgets as policy (ORGANISM-ARCHITECTURE §5):
  `[genome].budget` sets the org-wide default (basis tokens|usd, window
  daily|weekly|monthly UTC, cap, mode hard|soft); per-seat overrides live
  in the new `seat_budgets` table with per-field precedence — a row with
  `cap = 0` is the deliberate release valve ("this seat runs free even
  though the org caps"). Ungoverned when no row and cap 0 (the shipped
  default): nothing computed, nothing injected, byte-identical legacy.
  Metering READS the existing per-session accumulator
  (`employee_window_usage`: SUM(total_tokens)/SUM(estimated_cost_usd) over
  the employee's sessions in the window) — no second counter. Every turn
  injects a `<budget_state>` block with the remainder so the agent
  self-economizes before the cap; HARD mode refuses turns that start over
  the cap with a clear notice naming the seat, usage, cap, and the
  escalation path; SOFT warns and continues. Enforcement is turn-boundary
  v1 — mid-flight enforcement is a recorded follow-up, not this contract.
  Typo'd window values fail OPEN to daily (turns keep running; doctor can
  flag the value) — a silent refusal would be worse.

- Wave 3 — rolling compaction handoff (ORGANISM-ARCHITECTURE §3): every
  completed gateway turn upserts ONE rolling summary row per
  (employee, session) — new roll = previous roll + the turn's doings,
  never an aggregate; head-bounded at 8 KB (tail-kept, UTF-8-boundary
  safe). A continuation (restart / session switch / /resume — a real
  transcript reload, not a live conversation) begins from the roll inside
  a `<continuation_summary>` block; TTL is read-time against
  `[genome].session_summary_ttl_minutes` (owner range 15–30, default 30),
  so an expired roll means a fresh start and config changes apply without
  migration. Deterministic v1 roll (prior roll + final turn text) — no
  extra LLM call; LLM-compressed rolls remain a follow-up knob. Summary
  writes fail soft (warn + continue): a failed write degrades to raw
  transcript continuation, never a lost turn.
  F1 follow-through: the authority-tools battery moved from
  `tests/org_authority_tools.rs` to in-crate
  `src/tools/authority_tools_tests.rs` under cfg(test) — its 54 tests
  exercise `issue_grant` at hand-built grants and precise timestamps,
  which only the sealed `pub(crate)` seam reaches; integration-test crates
  are external and must mint through the approver.

- Wave 2 — sessions bind employees (ORGANISM-ARCHITECTURE §2):
  `gateway_sessions` gains `employee_id` (additive migration; legacy rows
  backfill to `premiere` on load — owner ruling: the default conversation
  talks to the premiere). The genome consult resolves the SEAT through the
  bound employee before the session id, so chat sessions run under their
  employee's policy/grants instead of an ungoverned `gw_<hash>` seat; cron
  runs are unchanged (their session id already IS the employee id). The
  employee's charter (org system prompt) rides the frozen prefix inside an
  explicit `<employee_charter>` block — appended after the base prompt,
  byte-stable while the binding holds, so the prompt-cache discipline is
  preserved. New `/session [employee]` gateway command (alias `/use`):
  no args reports the bound employee; with an id it validates against the
  registry (unknown ids list the known cast) and rebinds at the next turn
  start — binding is a conversation act, not an authority act; every tool
  consult still gates under the bound employee's genome row at run time.
  `Gateway::get_persistent_sessions()` exposes the persistent store the
  binding lives on (survives restarts); `SessionStore::entry_for_source`
  gives commands the entry under its real key (group/thread shapes
  included).

- The cold-start cast (ORGANISM-ARCHITECTURE §1, wave 1): `org/cast.rs`
  declares the organism's nine manifest seats — `premiere` (executive,
  org-lead), `chief-of-staff` (executive, reports to premiere — the
  same-department report that makes `premiere` compute as org-lead),
  `governor` (meta-governance, reports to premiere), `identity-warden`
  (identity, reports to governor), `compass` (strategy, reports to
  governor), `crew-chief` (crew, reports to governor), `hrmaster` and
  `dispatcher` (crew, report to crew-chief), and `dp-the-program`
  (execution, reports to premiere). The gateway's org-store init seeds
  them idempotently on first run: employee rows via INSERT-OR-IGNORE,
  reporting edges only for seats with no edge, and premiere's standing
  `Org`-scope grant minted once through `SeatApprover::mint_for` (the
  approver seam — no direct `authority_grants` writes, even at seed
  time). A topology that fails to compute premiere as org-lead aborts
  the seed loudly instead of minting. The seeder writes no
  `seat_policies` rows: seeded seats stay ungoverned until an operator
  policies them. Default cron specs are recorded in the manifest; the
  cron store has no register-without-executing path, so actual
  registration belongs to the execution wave.
- `employees.system_prompt` — the charter column (a session executing as
  an employee runs under its charter). Added via the org layer's
  additive `ensure_columns` reconciliation, so existing databases gain
  it on next open; cron-backfilled rows keep `NULL`.

- **Mouse injection for `operant tui debug simulate` (`--mouse`)**: the headless
  simulator can replay mouse events after the key sequence —
  `<action,x,y>` tokens (`left`, `right`, `middle`, `drag`, `up`/`release`,
  `scroll_up`, `scroll_down`; 0-based viewport coordinates). This un-parks the
  `selection-highlight` surface: `apply_selection_highlight` is mouse-written
  and style-only, so no scenario could reach it before. The corpus gains a
  `selection-highlight` scenario (120x40 + 80x24) whose style golden pins the
  58% accent background blend and the 32% white foreground blend. Five
  defects were fixed on the way, two of them live-UI bugs: (1) `render_app`
  published `last_selectable_area` *before* `jcode_ui::draw`, one frame stale
  and a zero-rect on the first frame — the first mouse click after startup was
  silently dropped; (2) a token-advance off-by-one in the `--mouse` parser
  hung any sequence of two or more tokens; (3) the simulation exit check
  ignored the mouse queue, exiting at frame one before any mouse event was
  consumed; (4) no closing draw on the simulation exit path — `terminal.draw`
  runs before the event pump, so the last event's state never reached the
  dumped buffer; (5) `App::debug_snapshot()` now exposes `selection_anchor`,
  `selection_focus`, `selection_text` plus comma-free row fields, so drag state
  is assertable from scenarios.

- **fix(channels) — telegram `strip_tool_call_tags` underscore alias:** the
  iter-664 sanitize relocation dropped the `<tool_call>` tag form from the open-tag array and close-tag match, leaking raw tool-call JSON
  into user-visible Telegram replies; pre-existing, surfaced when the gate
  grew `-p operant-cli` (feature unification compiles the telegram tests).

- `operant-core/src/doctor.rs` — ONE doctor engine on `AppConfig`
  (docs/ORGANISM-ARCHITECTURE.md §6 F2). Two divergent engines existed:
  `operant-runtime::doctor` ran on `schema::Config` for the gateway's
  `GET /api/doctor`, `operant doctor` ran its own checks on `AppConfig` —
  same intent, drifting check lists. The surviving engine runs the union:
  config semantics on the `[providers]` section (default provider, API key,
  model, temperature range, fallback chain, model/embedding routes), data
  root (exists/writable/disk) at `operant_home()`, environment (git, shell,
  HOME, curl), and CLI tool discovery (`operant_tools`). A drift-pin test
  pins the category order so the CLI and gateway paths cannot silently
  diverge again.
- `org/seat_policy.rs` + `org/seat_policy_db.rs` — seat modes, the
  precedence engine, the sqlite store, and the `SeatPolicySource` seam.
- `org/hierarchy_edges.rs` — reporting lines as an edges table feeding
  `Hierarchy::new`; cycle/dangling defects are reported, not repaired.
- `org/pending_requests.rs` — durable escalation queue with atomic
  pending→resolved transitions and an `expired_as_of` TTL sweep.
- `org/seat_authority.rs` — `SeatAuthority` (policy + grants + requests
  in one consult) and `SeatApprover` (approve→`issue_grant` with TTL tiers,
  deny, expire; `approver_of` routing with operator-literal fail-closed).
- Agent builders: `with_seat_authority`, `with_unattended`.
- `[genome]` config block: `unrestricted_default`, `grant_ttl_days`,
  `queued_cron_jobs_resolve_grants`.
- `tests/governance_escalation.rs` (4 end-to-end),
  `tests/seat_policy_run_path.rs` (5), 6+8+8 unit tests across the new
  stores, 2 new `cron_session_isolation` barrier tests.

### Fixed

- **iter-806 — TUI selection highlights only the text a copy carries.** Two
  painters ran: jcode's line-level `highlight_line_selection` (text columns
  only) was handed SCREEN columns, and a post-render pass then repainted every
  selected row edge to edge, lighting up the `1›` gutter, padding and tool
  rows. The cell painter is gone. `copy_selection_range` hands jcode's
  painter normalized text columns, and one predicate
  (`selection_line_selectable`) decides for both highlight and copy that
  interface rows are skipped. System and error cards are now interface rows,
  so a copied answer no longer carries them. Table cells that wrap merge
  back into their row instead of becoming extra TSV rows.

- **iter-805 — agentic core: tool-loop guardrails warn the MODEL and stop the
  turn cleanly; no guardrail text in answers.** Warnings were emitted as
  answer `Content`, so they glued onto the reply, and the model never saw them,
  which is why it kept looping. They now ride the tool result the model reads
  next (hermes `_append_guardrail_observation`), and the operator gets a new
  `AgentEvent::Notice` rendered as an annotation. A halt no longer fires the
  user-interrupt flag ("Interrupted by user" ×2, prompt restored). It ends the
  turn with a plain-language reply after the results are recorded. Preflight
  rejections carry their call's name (they pooled under tool `""`), and a call
  with no name is refused as `(unnamed)`. The TUI shows one error card per
  failed run (the event plus the returned `Err` made two), keeps the streamed
  partial answer on error, shows a user stop as a quiet "Turn stopped.", and
  tool rows show `✗` for failed calls and `○` while pending (both read `✓`).

- **iter-802 — TUI copy carries no chrome: system cards rejoin, code/quote/table
  copy as document text.** System and error cards are pre-wrapped by their
  renderer, so each row was its own logical line and a copied card broke
  mid-sentence. The rows now map back onto the authored lines
  (`BodyAcc::push_rows_mapped_to_source`). Code blocks copy without the
  `┌─ lang` / `└─` frame and `│ ` gutter, blockquotes without their bars,
  and tables as tab-separated cells with the `──┼──` rule dropped, so they paste
  as real cells in Docs, Word and Sheets.

- **iter-801 — TUI copy is clean document text (jcode raw-line model).**
  Drag-select copied the selection's edge rows from *wrapped* screen rows,
  so text broke wherever it soft-wrapped and gutter prefixes came along.
  Both endpoints now map through the frame's `wrapped_line_map` to
  (logical line, column), and whole logical lines are copied, as jcode's
  `copy_selection_text_from_raw_lines_base` does: a wrapped paragraph or prompt
  pastes as one line, without the `1›` gutter. Interface rows (tool/status rows,
  and the header, which carries no line map) are dropped when the
  selection also covers conversation text. A selection entirely inside them
  still copies them. Replaces the edge/interior split and the P1-6 reflow
  helper.

- **iter-800 — TUI: large pastes reach the model.** A paste of 3+ lines
  or more than 150 chars shows as `[Pasted ~N lines #K]` in the composer and
  is stored in `PromptInputState.paste_contents`, but nothing read the
  store back: the model received the placeholder text. `take()` now expands
  every placeholder from the store before the submission leaves the
  composer, so the turn, history and failed-turn restore all carry the
  real text.

- **iter-799 — agentic core: long streams no longer die at `timeout_secs`,
  and a drop always earns its retries.** `[client] timeout_secs` was applied
  as reqwest's *total* timeout on the streaming request, which covers the
  whole body: every turn that streamed longer than 60s (reasoning plus tool
  calls) was cut mid-body as `error decoding response body`. The run loop
  also wrapped the whole stream in the 120s `request_timeout` ceiling.
  Streams now use a client with no total timeout. The header wait is bounded
  by the model's effective timeout, and the body by a per-chunk idle limit in
  both the client (`ChatStreamResponse::with_idle_timeout`) and
  `process_stream`. A silent stream ends as the new retryable
  `Error::StreamStalled`. The 45s stream-retry budget was started at turn
  start, so a cut after 60s of healthy output gave up with "0 retries".
  It now measures one unbroken run of failures and re-arms on stream
  progress. Mock-SSE tests pin both: a 1.3s dripping stream outlives a 1s
  timeout, and silence ends as a stall.

- **iter-797 — TUI: thinking, text and tool rows no longer render twice while tools
  run (2026-10-13 duplication).** A ToolStart that reserved a slot for the
  live thinking/text spliced them into the settled Body *and* the Streaming
  section rendered them again, so everything above a running tool showed
  twice until the first tool settled. Reserved tool rows now render once,
  in the Streaming section below the live tails
  (`operant_model::live_tool_rows`); they return to the Body at their
  anchor when the boundary flush commits the tails. A flush that commits
  nothing (mid-turn `/thinking-display off`) now releases the reservation,
  so the tool row can no longer jump below the final answer.
- **iter-797 — TUI: drag-select copies what you selected, every time.** Release used
  to copy text scraped from the *previous* frame's painted cells — a fast
  drag (Drag…Up in one event batch) copied nothing, and a selection longer
  than the viewport copied only its visible part. The text is now extracted
  synchronously from the drawn frame on release, Ctrl+C and copy-mode
  copy (`render::selection::selection_copy_text`); interior lines keep the
  clean source reflow, which no longer merges lines across the Body /
  Streaming boundary.

- **iter-745 — BUGS.md S7 closed: the ping-pong rung classifies an
  identical-output alternation as no-progress, and result streaks hash
  through the canonicalising fingerprint.** Two fixes in
  `tool_guardrails.rs` (the merged controller), exactly where the S7
  filing prescribed them ("fix the semantics in the merged controller,
  not in the dead engine"): (1) the pre-execution ping-pong rung now
  consults the alternating pair's result streaks — when both tools'
  current streaks hold the SAME canonical fingerprint with count >= 2
  (proof that every call in the alternation returned the identical
  output; a single coincidental shared round cannot reach 2 on both
  sides), the warn/skip verdict classifies as `NoProgress` instead of
  `PingPong`, and the skip arm arms the both-tools
  regardless-of-args backstop so the next call to EITHER tool skips —
  the correct advice, since no arrangement of calls that all return
  the same output can finish the task. (2) `observe_result` hashes
  results through `result_fingerprint` (valid JSON → canonical
  `hash_value` with recursively sorted keys; anything else → raw
  `hash_str`), so a re-serialized-but-identical output no longer
  resets the no-progress streak.

  Tests (mutation-proven — reverting either half turns its test red):
  `alternating_tools_with_identical_outputs_escalate_as_no_progress`,
  `re_serialized_identical_results_share_a_fingerprint`, and the
  gate's negative
  `alternating_tools_with_distinct_outputs_stay_ping_pong`.
  tool_guardrails suite 45/45 (was 42); operant-core lib suite 2484
  passed / 1 failed — the failure is the peer's in-flight WIP test,
  documented in their iter-731 body and green at origin/main; this
  delta adds zero failures. Clippy clean on the touched file.

- **iter-738 — skills/meta-skill-creator: registry.py `resolve()` now falls
  back to the root-relative candidate only when the file-relative one does
  not exist:** the walk returned the first inside-root candidate without an
  existence check, so a valid root-relative link from a depth>0 node was
  reported dangling whenever a same-named file-relative shadow path was
  absent — contradicting the function's own docstring and the Rust parity
  port (`skills_tool.rs resolve_pointer`, which checks existence per
  candidate). Surfaced by the iter-5 live-tree migration as 4 false
  dangling-link errors in linkedin-marketing; fixed, mutation-proven
  (fallback / file-rel preference / dangling-signal preservation), and all
  7 migrated live trees re-battery green (`registry.py --check` 0 errors).
  Also lands the migration execution log (plan §6, per-tree node counts +
  hand-repair ledger) and BUGS.md S-1/S-2/S-3 rows for the corpus defects
  the stricter validator surfaced.

- **iter-733 — docs: the phantom "progress-oscillation" rung and the
  false row-4 premise are gone (outline v5 + config docs):**
  `AgentConfig::guardrail_exempt_tools`'s doc comment
  (`operant-core/src/config.rs`) and `operant.example.toml` both
  advertised the exemption list as covering "ping-pong / no-progress /
  progress-oscillation" — no such rung exists (`RepeatPattern` has 5
  variants, none is oscillation), so operators were told about a
  capability that was never built. The list now names the two ported
  rungs that exist. The v5 outline's row 4 is corrected in place with
  an appended audit note: the `no_progress/mod_tests.rs` suite named
  since v2 was upstream-only (never in-tree; the port plan declined to
  create it), `pattern_tests` holds 12 tests not 13, and row 1's
  "blocked on the peer's gateway_runner.rs WIP" note is retired — that
  overlay is a rolled-back copy (index blob = `3429363f^`, worktree
  reverts iter-704/iter-720 fixes), so both row-1 slices are unblocked.
  No code behavior changes; the pure-test remainder landed at iter-732.

- **Unblock main: `PathBuf` inline format capture in the cron seat-memory
  prompt** (`operant-core/src/cronjobs/scheduler.rs`): `{path}` →
  `path.display()` — committed in 5464764d, broke every `cargo check/test
  -p operant-core` build.

### Changed

- **iter-808 — essential commands wired, jcode-only commands removed,
  autonomous ledger restored.** These had been registered but replied "not
  wired": `/compact` (folds the agent's stored history with the store-safe
  micro-compaction, `OperantAgent::compact_history_now`; recent turns stay
  verbatim), `/cancel` (the Esc stop), `/transcript` (markdown into
  `~/.operant/transcripts/`, opened in the external editor), `/changelog`,
  `/refresh-model-list` (drops the catalog cache and re-fetches), `/auth`, and
  `/goals` (the autonomous `TODO.md` ledger). 40 jcode-only commands are
  unregistered and gone from `/help`: accounts and subscriptions, the
  `/z*` premium commands, dictation, screenshots, selfdev, split view,
  telemetry, catch-up, workspace, and similar. The repo-root `TODO.md` (deleted in
  iter-101, so every autonomous tick was skipping) is back with `## Implemented`,
  `## Pending`, and a parser-ignored `## Review` for owner decisions.

- **iter-797 — TUI reasoning display defaults to `current` (jcode semantics).**
  `[agent] show_reasoning = true` (and the unset default) now means: the
  turn's thinking commits where it streamed and is hidden once the next
  user prompt lands — previously `full`, which kept every turn's draft
  above every answer. `/thinking-display full` restores the old behavior;
  `/thinking-display` now reports the active mode.
- **iter-797 — `[tui] refresh_rate_ms` is documented as ignored.** It was never read
  (frame pacing is adaptive); it stays accepted so existing configs still
  parse under `deny_unknown_fields`.

- **iter-794 — docs: senior-developer handoff + AGENTS.md visual-layer
  discipline sections.** `docs/2026-10-13-tui-visual-layer-handoff.md`
  codifies the live duplication complaint (reasoning-draft interleave),
  what is already fixed (iters 780/784/789/790/792), the two open
  render-side suspects, the full file-path reference map (ours + vendored
  jcode), repro recipes, and process discipline. AGENTS.md gains a "TUI
  visual layer — architecture + working discipline" section (one-renderer
  invariant, mutable-Streaming rule, three-state reasoning mode + the
  process-global cell's test-serialization requirement, scroll-mode
  selection contract, tool-row anchoring) and a "TUI debug + live-repro
  tooling" section; the Rust Code Standards header now mandates reading the
  relevant `rust-best-practices` chapters (canonical copy at
  `/home/ishanp/.agents/skills/rust-best-practices/`) in the same turn as
  writing code. A duplicate iter-792 label exists on origin/main (peer
  raced the same number — `feat(iter-792)` vs `fix(iter-792)`); per the
  append-only rule both stay and this is the collision note.
- **iter-792 — reasoning display honors its config at last (jcode
  ReasoningDisplayMode contract): the "duplicated final message" was the
  model's reasoning draft rendering as body text.** Root cause chain (live
  repro: rivers query — numbered draft list above the same prose answer):
  (a) the display shim's `config()` was a `LazyLock` of hardcoded defaults
  that never read the user's TOML, so `reasoning_display()` was frozen at
  `Full` and `/thinking-display off` changed nothing visible; (b) the CLI
  DisplayConfig default `show_reasoning = Some(false)` silently overrode
  the core default `true` at layering; (c) `Off`/`Current` had no live
  behavior anywhere. Fixes: a runtime override cell bridges
  `[display] show_reasoning` (and `/thinking-display off|current|full`
  flips) into the shim resolver; `Off` skips reasoning accumulation
  entirely; `Current` is ephemeral per jcode's
  `anchor_current_reasoning_block` — the trace streams live (dim+italic,
  iter-780 sentinel) and never persists to the committed transcript;
  `Full` keeps classic behavior. CLI default now `None` (unset → core
  `true`, jcode parity). Regression contract test is single-test
  sequential (the override cell is process-global; parallel test threads
  race on it). Live-verified: thinking renders `rgb(100,100,100)`+italic
  distinct from the `rgb(200,200,195)` answer.
- **iter-790 — command backlog closed at the dispatch layer: no
  registered command can fall through to the model as prompt text.**
  Sweep was 48/106: 58 registered names (split, splitview, account, zzz,
  commit, swarm, …) neither the TUI intercept nor the command registry
  handled, so typing them submitted "/split" to the LLM as a prompt. Fix:
  a not-wired catch-all in BOTH dispatch loops (interactive + headless
  sim) — a REGISTERED name (hidden aliases included, via the new
  registered_command_names_including_hidden accessor) that no handler
  claims is consumed with an honest "/x is registered but not wired in
  operant yet — see /help" notice instead of silently paying a model
  turn. Unregistered text still submits normally. The headless sim now
  mirrors the interactive dispatch (registry fallback + catch-all), so
  command_sweep.py measures production behavior: **106/106 consumed.**
  Suite 1751/0; corpus 0-drift.
- **iter-785 — glitch 2 fixed: thinking that precedes a tool call now
  renders ABOVE the tool row.** Root cause: with P0-2's deferred flush
  (the AG|ENTS.md seam fix), a tool row settled into the Body section
  while its preceding thinking was still live in the mutable Streaming
  section — which renders BELOW Body — so the live viewport and emitted
  scrollback both showed tool-before-thinking, inverting the stream
  order. Fix (render layer, flush timing untouched): display_messages
  splices the live tails INTO the slot the ToolStart reservation
  promised — immediately before the reserved tool row (identified by
  after_index exceeding the committed messages). Once the boundary flush
  commits them, the reservation stops matching and the committed message
  takes over the slot — nothing double-renders. The seam test stays green
  in its ORIGINAL form (ToolStart still never flushes mid-response).
  Regression test: thinking_preceding_a_tool_renders_above_the_tool_row.
  Live-verified: read-file turn renders user → thinking → ✓ aft_read →
  answer. Suite 1751/0; corpus 0-drift.
- **iter-784 — glitch 1 fixed: no more shredded/duplicated streaming text
  in terminal-scrollback.** Root cause: the adapter pushed the LIVE
  streaming rows (thinking + text) into `display_messages`, which feeds the
  settled Body section — and the scrollback emitter treats every Body row
  as settled history. Each growing frame's partial text froze into native
  scrollback at its then-current width (`The` / `The wi` / `I` / `A fleeting
  g` — the exact shredding the user reported), then the viewport re-rendered
  the same indices. Fix (jcode's model): live rows render in the MUTABLE
  Streaming section only — `prepare_streaming_cached` now carries the
  thinking tail (sentinel-wrapped, dim+italic per iter-780) above the
  answer text; `has_streaming` opens the section during thinking-only
  phases too; the adapter no longer emits live rows into Body
  (`streaming_partials_do_not_pollute_display_messages` pins the
  invariant). Live-verified: poem turn → scrollback holds prompt,
  committed thinking, complete poem — no fragments; emit trace showed 0
  emissions during streaming. Suite 1750/0; corpus 0-drift.
- **iter-779 — glitch 3 fixed: thinking text now renders dim+italic
  (shaded) in BOTH the live stream and committed history.** Root cause:
  the adapter stored `streaming_thinking` and committed
  `ContentBlock::Thinking` rows RAW; the markdown renderer keys dim+italic
  off the REASONING_SENTINEL, so unmarked thinking rendered as plain body
  text. Live tail now wraps complete lines via `reasoning_line_markup` and
  the trailing partial via `reasoning_partial_markup` (no mode gating —
  the live tail always shows); committed rows route through the
  previously-dead-on-this-path `format_reasoning_markup` (mode-gated:
  Off/Current hide persisted reasoning, matching the reload path's
  existing semantics). Both adapter tests pinned to the sentinel
  contract; style goldens reshaped (C8C8C3 plain → 646464+italic dim),
  re-proven deterministic, corpus 0-drift. Suite 1750/0.
- **iter-778 — post-P4-2 remaining-gaps outline** (`docs/plans/plan-2026-10-12-tui-remaining-gaps-outline.md`):
  all four live complaints now closed; the queue reduces to the 62
  fall-through commands (four honest tiers: wire-only aliases, port-decision
  unregisters, real-feature seams, registry noise), the P4-6 slow-frame
  audit, and BUGS.md D-1 doc-debt. Supersedes the 2026-10-11 outline.
- **iter-774 — P4-2: the notification overlay system is purged — jcode's
  status-notice mechanism replaces it.** User complaint #1 (legacy
  notification overlays) closed: the every-response "Response complete ·
  /copy…" banner, the full-screen `⚠ Error` modal (44 border rows on a
  provider stream-death), and the countdown progress-bar machinery are
  all deleted. What replaces them: `set_status_notice` (text + Instant,
  jcode's `status_notice` contract — 3s TTL enforced in the accessor, no
  tick loop) and `push_notice` routing — Info/Success/Warning → status
  notice; Error → inline transcript card (`SystemMessageStyle::Error`,
  theme error color) + notice. All 26 `push_notification` call sites
  converted; errors no longer render as transcript rows AND a modal AND a
  margin-box (one source of truth per event). `notifications.rs` reduced
  to the `NotificationKind` taxonomy; `render_error_modal`,
  `NotificationQueue`, `error_modal_scroll_offset`, and the
  `OPERANT_FROZEN_NOTIFICATION_CLOCK` test seam are gone — which made
  `tool-block` provable again (its documented nondeterminism WAS the
  banner's wall-clock bar). Corpus: 20 goldens reshaped, `error-modal` +
  `notification-banner` scenarios deleted with their surfaces. Suite
  1750/0. Found and fixed mid-purge: the mechanical `status_message =
  Some(..)` conversion had rewritten `set_status_notice`'s own body into
  self-recursion (stack overflow in 3 tests).

- **iter-775 — v6 outline CLOSED: every queued row landed.** The
  outline that has driven this line's queue since iter-738 is now empty:
  row 3 (vision-routing run-path test, iter-763), row 5 (the
  `[observability]` seam + the cron outcome quartet, iter-772), row 6b
  (the progress-token stall rung, iter-773) and §2 P1-A (the
  per-channel dispatch breaker, iter-774) are all marked landed in
  `docs/plans/plan-2026-10-09-remaining-gaps-outline-v6.md`, each with its
  landed-shape note (what shipped, what proves it). Docs-only — no
  code changes.

- **iter-774 — v6 outline §2 P1-A LANDED: the per-channel dispatch
  breaker.** The saga this exists for: a dead token (401) followed by
  the gateway re-issuing the same send on every cycle — 401 → 400 →
  503 → 404, for hours, hammering a platform that already told us to
  stop. `Gateway` now mounts a `DispatchBreaker`
  (`gateway/dispatch_breaker.rs`) on both outbound seams
  (`send_to_platform`, `send_message_return_id`): after
  `OPEN_THRESHOLD` (3) consecutive failures for one
  (platform, channel, class) key, sends fast-fail at the seam **before
  the adapter is called** — the platform never sees the request, and
  the reason is logged once per open, not once per cycle. Design
  carried from the blueprint (`services/dispatch_breaker.py` #526):
  **separate namespaces per concern** (Transport / Auth / Wedged — a
  flaky network can never open the breaker for an auth-dead token);
  **capped exponential cooldown** (30s → ×2 per tier → 300s clamp);
  **exactly one half-open probe** (an atomic SQLite `UPDATE … WHERE
  probe_lock = 0` claims it — concurrent senders race and one wins),
  past a 30s probe deadline a never-reported probe is taken over so a
  crashed sender cannot wedge the channel forever; **fail-open on
  infra down** (a store error logs and the send proceeds — the breaker
  can never itself take delivery down). The classifier
  (`classify_error`) is hermes' anti-marker order: auth markers
  (401/403/unauthorized/…) match before transport markers, so a
  credential death is never misclassified as a network blip. State is
  SQLite-backed next to the app database (separate table) so a hot
  channel does not get a fresh breaker by restarting the gateway; an
  unmountable store degrades to unbreaked sends with a warn, never a
  boot failure. Tests: 10 pure-breaker (classifier order, cooldown
  clamp, open-at-3, success-closes, class isolation, per-channel
  isolation, exactly-one-probe, failed-probe re-open with stepped
  cooldown, successful-probe reset, stale-probe takeover) plus 3 seam
  tests with a counting failing adapter (open-breaker fast-fails with
  the adapter untouched; the id-returning seam shares the guard; an
  unmounted gateway keeps pre-breaker behavior byte-for-byte). The
  seam is mutation-proven: removing both pre-checks turns both
  seam tests red. Core suite 2513/2513; CLI compiles clean.

- **iter-773 — v6 outline row 6b LANDED: the progress-token stall rung**
  — the last row-6 item. The model-evasion shape the loop detector was
  blind to: iterations whose narration keeps *claiming* forward motion
  ("next…", "now I'll…", "step N") while **nothing executes** — every
  call guardrail-skipped — reworded each time so `IdenticalOutput`'s
  byte-identical signature never fires. New `RepeatPattern`
  `ProgressTokenStall { count }` plus a claim-vs-action streak in
  `ToolGuardrailTracker`: `observe_output` gains `claimed_progress`
  (a pure `text_claims_progress` matcher over the narration) and
  `acted` (the batch executed ≥1 non-skipped result); consecutive
  claims without an action warn at `PROGRESS_TOKEN_WARN` (4) and arm
  the shared skip backstop at `PROGRESS_TOKEN_SKIP` (5). Its reset
  semantics are its own: an all-exempt polling batch resets it
  (iter-688 parity), a claim *followed by an executed call* resets it
  (the normal agent rhythm), and it deliberately sees batches the
  identical-output rung discards — an all-skipped batch IS the stall
  shape. Both pattern-message fns and the armed-skip branch gained
  arms (the branch now reports the pattern that actually armed the
  skip). Wire-up: `observe_iteration_output` takes the narration,
  `run.rs` passes it; the Warn feed speaks the new copy. Six new
  tests (vocabulary, warn-at-4/arm-at-5 with the arming pattern,
  executed-call reset, no-claim-alone silence, exempt-batch reset,
  skip copy); mutation-proven — dropping the increment turns the three
  accumulation tests red. Guardrail suite 51/51; core suite 2500/2500;
  clippy clean in touched zones (net-zero new warnings).

- **iter-772 — v6 outline row 5 LANDED: the cron lifecycle quartet is
  observable through the gateway.** The queued seam closed: `AppConfig`
  gains the `[observability]` section (schema-parity type; `backend`
  defaults to `"none"` — dark-merged, byte-identical). The gateway
  constructs the observer through the runtime's canonical factory
  (`core_observer_from_config` — the existing core↔runtime bridge aimed
  in the direction the core-trait scheduler needs, so backend semantics
  cannot drift between surfaces), mounts it on the scheduler at both
  WriteBarrier arms — which makes `CronDeliveryQueued`/`Parked`/
  `Reclaimed` live in the gateway for the first time — and clones it
  into the delivery sender loop, which now emits
  `CronDeliveryOutcome` as the consumer-side report (redacted error text
  in the event; the ledger keeps the raw error for forensics).
  `CronDelivery` carries `job_id`, populated at both handoff sites, so
  the outcome event attributes the originating job without a ledger
  lookup. Covered by the two existing handoff tests (assert the job id
  on the first send and the replay) — mutation-proven: a blank job_id
  at each handoff turns its assertion red. The ledger stays the
  correctness contract; this row is observability only.
  `operant.example.toml` documents the section.

- **iter-770 — command backlog tier 3 (agent flows): `/improve`, `/judge`,
  `/refactor`, `/fix`, and `/triage` now launch real one-shot agent
  sessions instead of falling through to the model as literal prompt
  text.** Each resolves in the command registry to
  `CommandResult::ShellCommand`, arming the existing suspend-and-run
  escape hatch (`pending_shell_command` → `run_suspended_shell_command`,
  the /setup path): the TUI suspends and the current binary runs the
  session with inherited stdio. Flag shapes follow the CLI contract:
  `run --autonomous` rejects `--query`, so `/improve` (whose whole
  purpose is the TODO.md ledger) goes autonomous-only; `/judge`,
  `/refactor`, `/fix`, `/triage` take `run --query "<prompt>"`.
  argv[0] is `current_exe()` per the /setup precedent (a bare "operant"
  could resolve a different version on PATH). Handlers extracted into
  `register_agent_flow_handlers` so the registry they build is testable
  without entering the TUI; the regression test pins resolution + flag
  shape for all five. Headless consumption verified: all five fire
  SlashCommand events with no agent turn and no credential error.
- **iter-769 — command backlog tier 2 (dialog handlers): `/login`,
  `/init`, `/logout`, and `/observe` now intercept instead of falling
  through to the model as literal prompt text.** `/login` and `/init`
  open the connect dialog (the login surface); `/logout` opens it with a
  notice pointing at where keys actually live (the TUI holds no
  credential material to drop); `/observe` surfaces the latest
  `tool_use_block` (name + output preview) in the transcript until the
  side panel exists. `/models` and `/agents` already resolved via
  alias routing. Command sweep: 40 → 44/106 consumed. Regression test
  pins all three interceptions.
- **iter-768 — P4-5 selection pane-scoping: corpus now actually selects.**
  Diagnosis: the boot connect dialog (no credentials in the pinned sim
  config) swallows the first Left-Down — its gate treats the click as
  outside and closes the dialog, so the follow-on drag ran with no anchor
  and the committed selection-highlight golden had been pinning an
  UNSELECTED frame since the corpus was cut. The scenario now closes the
  dialog (`<esc>`) before the drag; asserts gain
  `selection_anchor != nil`/`selection_focus != nil` so an inert golden
  can never return silently. New `selection-highlight-chrome` golden pins
  the pane-scoping fix (iter-672 content-space points + Drag band clamp):
  a drag that runs from mid-band up into the banner and out to the far
  right paints only in-band transcript rows — banner rails/info widgets
  keep their own styling. Live-audit v1's 442-char/17-line chrome leak
  was measured pre-672; verified fixed headlessly (141 selection cells,
  rows 11-21 only).
- **iter-767 — P5-4: mode-aware corpus scenarios.** The scenario
  harness now supports a per-scenario `config` TOML fragment (schema
  field, appended to the pinned config) for behavior modes no key or env
  var can reach — terminal-scroll mode being the first consumer. Two new
  goldens pin the P5 surface: `terminal-scroll-mode` (inline viewport
  with the preview band gated off after a settled conversation) and
  `terminal-scroll-cls` (`/cls` arms the native-scrollback divider flag
  AND collapses the live window). `debug_snapshot` exposes
  `terminal_scroll_mode`, `scroll_emitted_rows`, and `scroll_cls_divider`
  for assertions. Headless scope note: the sim's fullscreen `TestBackend`
  makes `insert_before` emission inert, so scrollback history itself
  remains live-verified territory — the corpus pins the render-visible
  surfaces.
- **iter-766 — status-tier flashes are now visible in an idle session.**
  `status_message` renders only inside the mid-turn spinner lines
  (`ProcessingStatus::Connecting/Thinking`), so iter-765's `/model-status`,
  `/info`, and bare `/thinking` outputs were invisible exactly when a user
  would type them (no turn in flight). All three now also land a transcript
  annotation (the `/status` precedent) alongside the status flash. Sweep
  stays 40/106; the regression test now pins the annotation count.
- **iter-765 — command backlog tier 1 (status messages): `/model-status`,
  `/info`, and the `/thinking` alias now intercept instead of falling
  through to the model as literal prompt text.** `/version` and `/usage`
  already resolved via the registry fallback; these three did not.
  `/model-status` flashes model · provider · credentials · streaming
  state; `/info` flashes version · model · provider · title; bare
  `/thinking` (the registered surface name) routes to the thinking-display
  handler and reports current state. Command sweep: 37 → 40/106 consumed.
  Regression test pins all three interceptions.
- **iter-762 — /cls in terminal-scroll mode leaves a visible boundary in
  native history (the design doc's spacer).** The live-window collapse
  already worked (iter-761's capture proof); the missing piece was the
  history side: emitted scrollback is append-only, so a cleared view
  needs a divider between what came before and after. `/cls`/Ctrl+L now
  arms a one-shot divider — the scroll emitter prints one dim
  `view cleared — context kept ───` rule into native scrollback ahead
  of the next settled content. The watermark never resets (no
  re-emission, no duplicates). Live-verified in tmux: old reply,
  divider, then the next turn's prompt+reply.
- **iter-761 — P5-3 completion (re-ship): preview-band RENDER gate.**
  iter-759 gated the band's reserved line count but missed the render
  gate — the dimmed `N›` prompt row still drew at y=0 over the first
  content row in scroll mode's steady state. Both gates now carry the
  terminal-scroll check. Live-verified: the live viewport collapses to
  exactly the live tail + info widget + composer after each turn — no
  duplicated rows; history lives in native scrollback only.
  (Process incident, resolved: an earlier attempt at this fix was
  accidentally committed from the shared tree's index, sweeping a peer's
  in-flight notification-purge WIP to main as 8e72d80a — which did not
  compile. Reverted as 492e4028; the peer's WIP is preserved on the
  `fleet-hold-notif-purge-wip` branch for them to resume and land
  properly. The shared tree was left untouched — its working files carry
  their live edits.)

- **iter-759 — P5-3: previous-prompt preview suppressed in terminal-scroll
  mode.** The watermark floor makes `scroll > 0` the steady state in scroll
  mode, so the preview band permanently duplicated the last prompt above
  the composer — the same rows already emitted into native scrollback.
  The band is context for a scrolled-up reader; native scrollback IS that
  context now. Live-verified: after each turn the live viewport collapses
  to composer + status + info widgets only, history lives in the terminal.

- **iter-759 — P5-3: previous-prompt preview suppressed in terminal-scroll
  mode.** The watermark floor makes `scroll > 0` the steady state in scroll
  mode, so the preview band permanently duplicated the last prompt above
  the composer — the same rows already emitted into native scrollback.
  The band is context for a scrolled-up reader; native scrollback IS that
  context now. Live-verified: after each turn the live viewport collapses
  to composer + status + info widgets only, history lives in the terminal.

- **iter-758 — reasoning-alias collision salvage: the "missing output"
  mystery closed for real.** Live diagnosis (file-level capture of dropped
  SSE payloads) found omp/poolside-laguna emits OpenAI's full reasoning
  triad — `reasoning` + `reasoning_details` + `reasoning_content` — in ONE
  delta. Our `StreamingMessageDelta` aliases all three spellings to one
  field, so serde hit "duplicate field" — a HARD error that silently
  dropped every reasoning-bearing chunk (content included) since forever.
  That is what iter-754's drop counter surfaced (the 109/379 warnings were
  real corruptions: ours, not the provider's framing). The salvage path
  rebuilds the delta keeping the preferred alias and re-parses; regression
  tests pin both collision shapes. Live-verified against omp: ZERO drops,
  and laguna's reasoning traces now RENDER in the TUI — omp does emit a
  reasoning channel; we had been discarding it.


- **iter-757 — anthropic adapter compiles again (iter-754 follow-up).**
  The two `StreamChunk` literals in `parse_sse_event` were missed when
  `dropped_events` was added — invisible to every standard gate because
  the `anthropic` cargo feature is off by default and neither the core
  lib suite, the no-default-features TUI suite, nor the default-feature
  release build compiles that file. Both literals now carry
  `dropped_events: None`; `cargo check -p operant-core --features
  anthropic` is green. (Caught by cross-model review — the third
  consumer crate sweep after operant-runtime in iter-755.)

- **iter-756 — P5-2: the transcript now emits into native terminal
  scrollback.** In terminal-scroll mode (`tui.terminal_scroll_mode = true`),
  settled rows — committed messages, tool rows, system annotations — are
  printed into the terminal's own buffer via `Terminal::insert_before`,
  tracked by a last-emitted-row watermark so each row is emitted exactly
  once. A frame's emittable prefix stops at the first mutable section
  (Streaming/Reasoning/BatchProgress); the watermark advances as messages
  commit at flush boundaries, and the live viewport floors its window at the
  watermark — emitted history lives in the terminal, the live window holds
  only the unsettled tail. Live-verified: prompt + reply rows accumulate
  above the inline viewport, selectable in tmux copy-mode, while the
  viewport keeps status + composer + streaming. Resize re-syncs the
  watermark (history keeps its original wrap — native scrollback cannot
  reflow). Ctrl+L//cls deliberately does NOT reset the watermark: emitted
  history is append-only; a reset would duplicate the whole transcript.
  Also: incremental UTF-8 decode in the SSE stream reader — a multibyte
  char split across byte-chunk boundaries used to fail `String::from_utf8`
  and silently DROP the chunk (corrupting framing); the partial tail now
  waits for its continuation (regression:
  `chat_streaming_survives_utf8_char_split_across_chunks`). Corpus: the
  4-drift peer baseline is unchanged; zero new drift in default mode.

- **iter-754 — corrupted-SSE drop surfacing + a real parser tail bug
  fixed.** The duplication investigation's final answer: the parser
  NEVER ate event-duplicated deltas (proven live: a doubled
  keepalive+content+finish stream renders "perspersimimmonmon" — every
  delta faithfully appended, zero drops). What it DID eat silently:
  payloads with corrupted SSE framing (merged data lines / concatenated
  JSON — what a naive line-level replay hop emits). Two fixes: (1)
  `ChatStreamResponse` now COUNTS unparseable payloads; the openai
  adapter chains a terminal `StreamChunk{dropped_events}` after the
  source ends; `process_stream` emits ONE `AgentEvent::StreamCorrupted`
  (only when drops>0); the TUI surfaces "⚠ N corrupted stream event(s)
  dropped by the provider — output may be truncated". Live-proven via a
  line-doubling corrupter proxy against real omp: `StreamCorrupted
  {dropped_events: 2}` in the event dump, content intact. (2) REAL BUG:
  the stream-end branch parsed only ONE remaining buffered event — with
  the body fully buffered (typical), one corrupted payload silently
  discarded the ENTIRE valid tail (regression test:
  `chat_streaming_counts_corrupted_sse_payloads_as_dropped` — valid
  events after corrupted ones now yield; before the fix the valid tail
  was lost). Empty turns keep iter-752's status message.

- **iter-753 — P5-1: terminal-scrollback mode exists — the TUI can run in
  the main buffer.** New `tui.terminal_scroll_mode` (default OFF; documented
  in operant.example.toml): when enabled, `TuiApp::enter` skips the
  alternate screen entirely and constructs the terminal with
  `Terminal::with_options(TerminalOptions { viewport:
  Viewport::Inline(14) })` — the app renders in a bottom strip of the main
  buffer and everything above is the terminal's NATIVE scrollback (tmux
  copy-mode, native selection, copy across the whole session). Mouse
  capture stays off in this mode so the terminal's own selection works.
  Live-verified end to end: boot, prompt submit, streamed reply
  ("Hello from Operant"), composer, status row and info widget all
  functional with no alt-screen; exit/teardown paths unchanged (the
  leave-alt-screen sequence is a no-op in the main buffer). This is the
  P5-1 milestone from the 2026-10-10 design doc: the terminal handoff.
  Transcript emission via `insert_before` (native history accumulation)
  is P5-2; the cramped 14-row viewport is the expected milestone state.

- **iter-752 — a corrupted provider stream no longer ends the turn
  silently.** Forced-repro findings (double-emitting SSE proxy against the
  real omp omniroute gateway): the live chat provider is omp
  (localhost:20129, OPENAI_BASE_URL env precedence — the earlier kilo
  attribution and its "no reasoning channel" probes are RETRACTED, they
  were 401s misread as clean streams), and a doubled-delta stream kills
  the turn INSIDE the agent — Done{content: ""} with zero Content events
  (event-dump proven), an empty assistant row persisted, and the TUI
  rendered nothing at all. The Done arm now surfaces it: a turn that ends
  with no message and no text sets the status notice "Empty reply — the
  provider stream may be corrupted; retry or switch model" instead of
  failing blank. The deeper fix (where the doubled deltas die in the
  agent's stream pipeline before becoming Content events) is recorded as
  the next core-side investigation. Unit regression:
  an_empty_done_turn_surfaces_a_corrupted_stream_notice. The three
  dialog goldens whose harness scripts terminate with done{text:""}
  re-shape intentionally (the notice strip now appears). Corpus delta:
  zero beyond those (4 pre-existing journey/skills drifts reproduce at
  origin/main without this change).

- **iter-751 — docs: P5 terminal-scrollback design + the duplication
  investigation record.** The owner re-stated the requirement as native
  terminal scrollback (select-and-copy across a long conversation), which
  supersedes the jcode-parity framing — jcode is alt-screen and cannot
  serve it. The design grounds in ratatui 0.30's own first-class support
  (verified in the vendored source): the no-alt-screen init variant
  (init.rs:416), `Viewport::Inline(n)`, and `Terminal::insert_before` —
  ratatui's doc example is literally a streaming chat. Five waves
  (P5-1..P5-5), mode-gated first, corpus impact scoped. Also records
  today's duplication investigation (persisted transcripts clean, frame
  captures clean, handlers re-verified; remaining suspects: pre-744
  session inode, gateway double-emit; forced-repro blocked by the NEW
  finding that `-c <config>` is silently ignored for provider selection
  by chat/run — itself filed as the next fix), and the command-sweep
  backlog tiers.

- **iter-750 — command sweep: /cls + /clear-view implemented, alias tier
  wired, the lister fixed, the sweep committed.** The sweep (every
  REGISTERED command driven through the headless simulator, asserting
  consumption) found 73/106 registered commands falling through to the
  model as literal prompt text — the "/commands not working" class.
  This iteration lands the provable tier: /cls and /clear-view now run
  the terminal-style view collapse (jcode semantics: view only, context
  kept — NOT /clear, which drops the conversation; exact Ctrl+L sequence,
  no post-capture invalidate); the alias tier routes /commands → /help,
  /models → /model, /split + /split-view → /splitview; the empty
  `tui debug slash-commands` lister (a stub since iter-154) now prints the
  REGISTERED_COMMANDS registry with a consumption caveat; and
  command_sweep.py ships as the standing instrument (37/106 consumed
  after this wave; the remaining ~69 are the handler backlog — each
  needs its jcode-parity surface, tracked in the sweep output).
  /copy verified working live (wl-copy → wl-paste round-trip); its
  near-invisible confirmation is the P4-2 notification purge's surface.
  Unit regression: cls_clears_the_view_only_and_the_alias_tier_routes.

- **iter-749 — P4-4: thinking display on by default, `/thinking-display`
  toggles it — jcode parity.** The vendored tree already had the full
  reasoning-trace renderer and the shim already resolved
  `ReasoningDisplayMode::Full` from jcode's `show_thinking = true` default;
  what was missing was the runtime gate and the command. Added
  `App.thinking_display_on` (initialized from the shim's
  `display.show_thinking`, default ON), a `TuiState::show_thinking()`
  accessor, a gate on the reasoning-row preparation in `ui_prepare` (hidden
  traces still exist as display messages — history is not rewritten, they
  just prepare zero rows), and `/thinking-display <off|full|current>` (the
  vendored tree implements one trace mode, so full and current alias;
  jcode's muscle-memory words are kept). Unit regression:
  thinking_display_defaults_on_and_the_command_toggles_it. Reminder from
  the audit: the configured kilo gateway emits NO reasoning channel under
  any request dialect (4 raw SSE probes), so visible thinking also needs a
  reasoning-forwarding endpoint — the render side now renders whatever
  arrives, by default.

- **iter-746 — P4-3: the todo list is a live surface.** The audit's
  "half-assed implementation" verdict, closed: the tool, the renderers, the
  types and the `/todos` registration all existed, but the event carried
  counts only and no store ever held the items — the TUI could render
  nothing but "Todos: 0/2 done". The ToolComplete(todo) path now parses
  the full list from the tool's result JSON (the same source the counts
  event reads) into `App.todos` and builds the TodoCardPayload, the
  info-widget Todos band and the pinned transcript card render it live
  (verified live: "✓ fix scroll / ○ fix todos" card with progress pips),
  and `/todos` toggles the pinned card band. The vendored TodoItem gains a
  [port-adaptation] serde default for `priority` (upstream always emits
  one; operant's tool result names only id/content/status). Unit
  regression: todo_tool_result_populates_the_live_store_and_todos_command_
  toggles_the_band. The counts one-liner stays as the status-line signal.
  Label note: peer took 745 mid-flight; this ships as 746.

- **iter-744 — P4-2.5: a retried stream no longer duplicates the reply.**
  Root cause (v2 outline, live-audit): `AgentEvent::RetryScheduled` re-armed
  the turn but never cleared `streaming_text`/`streaming_thinking` — the
  died attempt's partial text stayed buffered, and since the provider
  restarts the response from its top, the retry's first delta APPENDED to
  the partial: the user watched the reply render twice. On this provider
  (stream-death flake is the org's known condition) that is the most-hit
  duplication path in the field. The fix discards the died attempt's text
  at the retry boundary; the turn stays live (`is_streaming` untouched —
  mid-turn retry display semantics are pinned by an existing test). Unit
  regression: retry_must_not_append_the_resurrected_stream_to_the_died_
  partial (partial -> RetryScheduled -> full text asserts replace, not
  append). Corpus delta: zero (the 4 journey/skills drifts pre-date this
  change and reproduce at origin/main without it).

- **iter-743 — P4-1 follow-up: ghost-scroll-offset clamp + live scroll
  geometry in the F12 debug overlay.** The reflow-anchor growth correction
  counts the streaming bubble's rows, but the flush boundary (bubble ->
  committed message) collapses them: the reader's offset kept rows that no
  longer exist (live-measured: offset 355 against a 15-row max after one
  streamed turn — the view parked correctly since the render clamps its own
  paint, but PageDown had to walk 34 ghost rows home). `reconcile_scroll_-
  anchor` now clamps the STATE to the frame's live max (0 = no frame
  rendered, so unit tests without a renderer are untouched) and voids the
  pin. The F12 debug overlay gains Scroll (offset / max / total wrapped
  lines) and Follow (tail|paused + resolved) rows — the live instrument the
  P4-1 audit said it needed: the dead ladder was diagnosed with ten blind
  probes where these two lines would have named the override immediately.
  Unit regression: reconcile_clamps_ghost_offset_after_the_flush_shrinks_-
  the_transcript (offset 355 + live max 15 -> 15). Live: PgUp x2 moves the
  viewport to the transcript top with overflow (verified on the instrumented
  build); corpus delta: zero (the 4 journey/skills drifts pre-date this
  change and reproduce at origin/main without it).

- **iter-741 — docs: jcode-parity gap outline v2 — root causes pinned from
  source.** Deep audit pass over the four still-live complaints: (1) todo
  one-liner — AgentEvent::TodoUpdated carries counts only while the full list
  JSON sits in result.content; jcode's TodoEvent carries the whole
  Vec<TodoItem> (jcode-base/src/bus.rs:46); store + InfoWidgetData feed +
  /todos handler are the missing wiring. (2) duplicated streaming —
  RetryScheduled never clears streaming_text/is_streaming
  (tui/app/agent_events.rs:472), so a died attempt's partial text appends the
  retry's full re-stream on this stream-death-prone provider; one-line fix +
  retry-dedup corpus scenario. (3) scroll — jcode confirmed alt-screen
  (ratatui-0.30.2 init.rs:400 EnterAlternateScreen via src/cli/terminal.rs:353
  ratatui::init); "integrated" scroll = an internal ladder that never dies
  (prompt jumps, bookmarks, tail catch-up, resize anchors) + optional
  handterm host scrollbar; native-scrollback is a distinct P5 wave if the
  owner still wants it after the evidence. (4) thinking — jcode default
  show_thinking=true + /thinking-display off|full|current + /effort; operant
  render chain complete, kilo emits no reasoning under 4 flag probes
  (thinking.type, reasoning_effort, 3 models). Order: P4-1 scroll seam ->
  P4-2.5 retry dedup -> P4-3 todo -> P4-2 notifications -> P4-4 thinking.

- **iter-740 — docs: second live-audit gap outline (jcode-parity v1).**
  First-hand tmux feedback loop on the iter-731 deploy: the scroll ladder is
  verifiably dead in the field (8x PageUp / Up / Alt+Up / Ctrl+Up, zero
  transcript movement, content proven above the fold, F12 proves keys arrive,
  suite pins the state machine -> render-seam loss, zero scroll-key corpus
  coverage); notification surfaces still stack (error modal + transcript rows
  + margin box + every-response banner); selection paints whole-frame chrome
  (442 chars / 17 lines for a 5-line conversation); the todo data path is
  deliberately stubbed while tool/renderers/commands all exist; thinking is
  provider-absent (3 raw SSE probes: no reasoning_content from kilo
  small-stack / deepseek-v4.1-flash / glm-5.3-flash even with the thinking
  flag). REFUTES the old P2-8 premise: jcode is also alt-screen
  (src/cli/terminal.rs:353 ratatui::init()) - there is no native-scrollback
  port; the real work is waves P4-1..P4-6 (scroll fix first).

- **iter-737 — outline v6 §1 row 2 (NEXT SLICE) LANDED: oversized tool
  results offload to a durable workspace artifact with a TOC stub.**
  iter-729 wired `[agent] max_tool_result_chars` (head 2/3 + tail 1/3)
  but left the model with a dead-end marker — the truncated middle was
  gone with no retrieval path (the original 26%-context-burn fetch).
  The runtime Agent's ingestion (`results_for_ingestion`, both turn
  paths — plain and streamed) now writes any result over the cap to
  `<workspace>/.operant/artifacts/tool-results/<sha256[..16]>-<tool>.txt`
  (content-addressed, so a repeat reuses the artifact) and ingests a
  one-line TOC stub naming the path and the `file_read` retrieval
  semantics (`offset`/`limit` are 1-based LINES). The artifact lives
  inside the security policy's allowed root, so the agent's own tools
  can page back through it; the observer and TurnEvent keep the full
  output, as before. Best-effort and dark-mergeable above the
  threshold: an unwritable workspace falls back to the iter-729 trim
  (marker + head/tail), and `max_tool_result_chars = 0` still disables
  everything. Tests: stub + byte-identical artifact round-trip, the
  zero-disable passthrough, and the unwritable-workspace trim
  fallback — operant-runtime lib suite 1641/1641.
  `ponytail:` artifacts accumulate with no retention sweep — a GC
  slice owns cleanup once the shape proves out.

- **iter-734 — outline v5 §1 row 1 LANDED (both slices): the gateway
  delivery consumer settles its ledger row, and the seat authority
  carries the employee registry.** The row's "blocked on the peer's
  `gateway_runner.rs` WIP" note was stale — the iter-733 audit proved
  that overlay is a rolled-back copy (index blob = `3429363f^`, worktree
  reverts iter-704/iter-720 fixes), not an active build — so both
  slices were unblocked and land together in `start_gateway`.
  **(a) Consumer mount** (`crates/operant-cli/src/gateway_runner.rs`):
  the cron sender loop discarded `delivery_id`; it now settles each
  `CronDelivery` via `CronDb::mark_delivery_outcome` — success is
  terminal (`'delivered'`), failure records the error and leaves the
  row pending, where the 2h reclaim replays it. Rows no longer linger
  until the replay (correct but wasteful). **(b) Registry attach**:
  `SeatAuthority::new` gains
  `.with_employee_registry(Arc::clone(&employee_registry))` — Bounded
  delegation now resolves the employee row it delegates INTO instead of
  failing closed on the registry-unavailable arm (latent today: no
  production row sets Bounded yet). `ObserverEvent::CronDeliveryOutcome`
  stays unemitted — the gateway holds no observer handle (grep: zero
  matches) and the ledger is the contract that matters. The trinity
  P1-A per-channel dispatch breaker rides this path next; its hook is
  `gateway/mod.rs:526 send_to_platform`, not the cron loop.

- **iter-731 — P3-10 (2026-10-09 live-audit plan): DNS failures say what
  they are; the live config migrates off the retired `igs` name.** The
  SSRF pre-flight's fail-closed DNS arm surfaced raw resolver text
  ("Tool error: URL safety check failed: IO error: failed to lookup
  address information: Name or service not known") — reproduced live
  when `web_extract` hit a host-DNS-dead domain; the message read like an
  operant bug when it was the environment. The arm now names the actual
  condition (hostname does not resolve from this host, check
  network/VPN/DNS; the check stays fail-closed). Separately, the live
  user config carried `preferred_provider = "igs"` (the retired name,
  working only through the iter-716 alias fallback) — migrated to
  `"sourcehound"` on the host (backup: `operant.toml.bak-20261009`), so
  the native chain is the configured default and not a fallback.
  Suite note: the shared tree carries peer in-flight `config.rs`/agent
  WIP that fails its own `agent_block_parses_guardrail…` test locally;
  origin/main proves green in a clean worktree — this delta
  (`security.rs` message arm only) is disjoint from that break.

- **iter-730 — P1-7 (2026-10-09 live-audit plan): the rotating 💡 hint
  rail is off by default, and the corpus pins the prompt-entry fade.**
  The vendored hint widget (jcode's info_widget_tips) rendered rotating
  keyboard tips into the status strip unconditionally — the visible half
  of the live audit's complaint 1 ("legacy overlays of notifications").
  New shim `display.show_tips` (default **false**, port-adaptation —
  upstream shows them unconditionally) gates all three render sites
  (ui_input ×2, ui_status ×1); the vendored cycle function and its tests
  stay verbatim. Also deleted the dead `adapter_types/tips.rs`
  (`select_tip`, #[allow(dead_code)], zero callers since iter-106's stub
  era). Determinism enabler: the prompt-entry fade — a wall-clock-driven
  blend on freshly-entered user prompts — joins the
  `OPERANT_DISABLED_ANIMATIONS` disable list (same corpus pin env, same
  config knob as the idle animation); its golden-captured mid-fade phase
  was the load-dependent style flake in `user-message-commits` /
  `first-message` (proven: with the pin, both re-proved deterministic and
  verify passes repeatedly; the goldens now hold the settled colors).
  Label note: origin carries two iter-728s (peer's max_tool_result_chars
  + this track's P1-6 selection copy, concurrent pushes) — append-only,
  both stay; this slice takes 730 after the peer's 729.

- **iter-728 — P1-6 (2026-10-09 live-audit plan): transcript copy is
  text-editor-clean — wrapped fragments rejoin their logical line with the
  source spacing, and tool-status chrome drops out of the copied text.**
  The drag/copy extraction was cell-based: every hard-wrapped screen row
  copied as its own line with wrap padding baked in, and a selection
  spanning a tool row copied the `✓ tool · N tok` status row as text.
  Interior fully-covered lines now copy from the pre-wrap source via the
  chat frame's `WrappedLineMap` (`reflowed_selection_text`): fragments of
  one logical line merge back into that line, original spacing preserved,
  and lines owned by a `tool` display message are excluded (message-owner
  resolved through `message_boundaries`; when boundaries and the display
  list disagree the filter degrades to include-everything rather than
  misclassify). Drag-edge lines keep exact partial-column cell extraction.
  The visible highlight is unchanged — every cell still paints with the
  pinned blend; only the copied text changes. 3 unit tests (fragment
  rejoin, chrome exclusion + logical-line separation, no-map fallback);
  corpus verify 0-drift (the selection-highlight golden pins paint, not
  copy text); suite 1747/0.

- **iter-727 — N-2 (2026-10-09 live-audit plan): a submitted message
  commits to the transcript immediately — all three submit paths bump the
  display-cache version, and the corpus simulator is now faithful to that
  contract.** The display cache is keyed on `transcript_version`, but none
  of the three submit sites (live loop, `submit_user_message`, headless)
  bumped it after pushing the user Message — the sent message was
  invisible until the first agent event arrived, the idle orb kept
  painting over it (`has_started_conversation` reads the same stale
  cache), and the earlier "PageUp blanks the transcript" repro was this
  staleness, not the scroll (the renderer already clamps
  `scroll_offset` — re-verified live on the deployed binary; the scroll
  ladder stays as-is). The mock path also dropped every submitted message
  into restore-to-composer (simulating an agent-init failure no scenario
  ever had), so no corpus golden could pin the submit-path commit — the
  mock now pushes + invalidates like the live paths. New scenario
  `user-message-commits` (empty agent script: the only way the message
  can render is the submit path's own invalidate) is red on the pre-fix
  code; `first-message` intentionally reshapes (user row + reply = 2
  messages; assert updated). Verify 0-drift; suite 1744/0.

- **iter-726 — P0-1 (2026-10-09 live-audit plan): a batched
  `text<Enter>` submits again — the paste-burst detector no longer swallows
  the Enter key.** Any latency-batching layer (tmux, SSH, mosh) can deliver
  the typed characters and the Enter in the same zero-timeout drain window;
  the old detector absorbed that trailing Enter as a literal newline, so
  the message sat in the composer unsent with no error — reproduced
  first-hand via tmux send-keys (100% swallowed) vs. human-cadence typing
  (0%). The drain core is extracted into `collect_paste_burst` with a
  queue-seam so the semantics are unit-testable (the corpus simulator
  bypasses the burst path, so the unit tests are the only gate).
  Discriminator: an Enter with more events behind it is a paste's embedded
  newline (absorbed, Windows Ctrl+V multi-line paste unchanged); an Enter
  that ends the burst is the user's submit key — stashed in `pending_key`
  and replayed through the normal key path, so the text lands and the
  replayed Enter sends it. 3 unit tests pin the three cases. Suite 1744/0;
  corpus verify 0-drift (the sim injects keys directly, bypassing the
  burst path).

- **iter-725 — P0-2/N-3 (2026-10-09 live-audit plan): tool rows no longer
  split a streaming sentence; the notification clock gained a corpus
  determinism seam.** ToolStart no longer flushes the streaming assistant
  text mid-response — providers interleave content deltas around tool_call
  deltas (observed live splitting "AG|ENTS.md" across the tool row), so the
  text buffer now spans the whole iteration and the seam joins
  automatically; tool rows anchor one slot further (`+ text_slot`) so they
  still render after the text that was in flight when the call was parsed.
  ToolComplete/ToolError are the iteration boundary and flush there —
  consecutive model responses never glue into one message. Tool-row dedupe
  by `tool_call_id` re-verified by unit test (the live "duplicate
  web_search row" was two model calls, not a rendering dup). New corpus
  scenario `tool-split-stream` gates the regression end-to-end (the
  contiguous "AGENTS.md and summarize." row is red on the pre-fix code);
  3 unit tests pin the three event-arm semantics. Enabler:
  `notifications.rs` `now()` seam + corpus pin
  `OPERANT_FROZEN_NOTIFICATION_CLOCK` — the banner's shrinking countdown
  bar was the documented wall-clock drift that made first-message/
  tool-block flaky and any banner-bearing golden nondeterministic; with
  the pin the bar renders full-width, byte-stable. Full corpus re-proven:
  **62 deterministic goldens / 0 unstable / 0 broken** (9 banner-bearing
  goldens intentionally reshaped by the frozen-bar pin). Suite 1741/0.

- **iter-718 — Clean-sweep 8: provider-less sessions derive their auth slots
  from the credentials that actually exist; first-message transition pinned
  end-to-end.** Completes the auth-aliveness fix for the model-only config
  shape (the user's omp session, the corpus sims): when no provider id is
  derivable, `auth_status()` now reports per-provider slots from env keys and
  auth-store entries instead of collapsing to nothing — so the header auth
  inventory shows the configured session rather than the full
  "nothing configured" fallback list. New corpus scenario `first-message`
  (send first message → transcript carries it + the reply; the auth circle
  for a keyed provider is filled, never hollow): the regression gate for the
  "opening screen never transitions" complaint. Full corpus re-proven: 61
  deterministic goldens (57 header goldens intentionally re-shaped by the
  configured-session inventory; 0 unstable). Suite 1738/0 incl. the
  store-based derivation test (env-mutating test removed — its window
  poisoned parallel App inits through AuthStore::load's env snapshot,
  breaking the Ctrl+A picker test's runtime assumption). Label note:
  origin carries a second iter-714 (micro-compaction, concurrent line) —
  append-only, both stay.

- **iter-716 — Clean-sweep 6: integration-truth fixes — `tool_backend` stops
  mis-advertising the live web_search chain.** The introspection tool claimed
  `tavily, exa, searxng, ddg` with `tavily` current — but the live chain is
  `sourcehound → tavily → exa → ddg → searxng` with `sourcehound` the config
  default and DDG the key-free fallback. Map + description now name sourcehound
  and default to it (static claim of the config default, noted in source).
  Stale `igs`-alias comment in web_tools.rs corrected (the config default is
  `"sourcehound"`, not "the old string"); the alias itself stays — pre-re-home
  configs depend on it reaching the key-free engine. NOT deleted (audit
  candidates re-verified as live): operant-tools `WebSearchTool` + routing feed
  the runtime agent registry consumed by the ACP orchestrator; the obscura/
  lightpanda names in browser_provider are intentional retired-name fallbacks
  with a pinned warning test. Suite: tool_backend 3/3 on clean origin/main
  (the shared tree is red from a peer in-flight core change — collect_map_nodes
  — unrelated to this delta).

- **iter-714 — Clean-sweep 5: composer text-area selection (jcode parity).**
  Shift+Left/Right select characters in the composer; Shift+Up/Down extend
  across visual rows (vim Visual mode keeps its existing Shift+arrow path);
  the selection renders reverse-video through the vendored wrap renderer
  (ui_input::wrap_input_text splits each wrapped segment into before/selected/
  after spans); Ctrl+C copies the composer selection and suppresses exit-confirm
  while one is live; any other edit key collapses it. The vendored surface
  gains `TuiState::input_selection()` (default None — no other implementor
  affected). Multibyte-safe via char-boundary clamping. Suite 1745/0 incl.
  3 selection tests (chords, backwards anchor, reverse-video render with
  surrounding chars intact).

- **iter-710 — Clean-sweep 4: Ctrl+Up/Ctrl+Down fine transcript scroll
  (jcode parity).** Completes the keyboard scroll ladder: ±3 on Ctrl+arrows,
  ±10 on PageUp/PageDown, ±20 on Alt+arrows, mouse wheel otherwise. Without
  the fine rung a mouseless session (tmux without mouse-mode, plain tty —
  wheel events arrive as nothing) could only jump in 10-line quanta; part of
  the "chat window is pinned" complaint (2026-10-09 visual audit). Both
  chords pause tail-follow; reaching the bottom resumes it. Suite 1742/0
  incl. chord-step regression test.

- **iter-709 — Clean-sweep 3: `last_msg_area` republished — the dead right-click
  context menu lives again.** The iter-648 chrome cutover deleted the
  dispatch-row writer that fed `last_msg_area`; every consumer (right-click
  message menu bounds, Ctrl+Shift+M cursor-open menu, menu clamping) gated on
  a zero rect and could never fire — production-dead since the cutover,
  flagged by the 2026-10-09 visual audit. The render pass now publishes it
  from the vendored `message_area()` alongside the other three hit-test cells
  (same post-draw discipline as the iter-661 fix). Pinned by a default-zero +
  setter-semantics unit test. Corpus verify 0 drift; suite 1741/0.

- **iter-707 — Clean-sweep 2: onboarding welcome-takeover phantom excised.**
  The ported draw carried a constant-false gate for a first-run welcome
  takeover (`onboarding_welcome_active` → early-return that suppresses all
  chat chrome), plus its ~140-line support stratum: the two trait methods,
  `OnboardingWelcomeKind` (4 variants), `LoginImportPrompt`, `ImportSummaryPill`,
  `TelemetryChoice`, `LoginImportRow`, a `#[cfg(any())]` test draw hook, and
  the onboarding margins branch. The upstream onboarding module
  (tui/onboarding.rs) was never vendored and no implementor ever opted in —
  the gate guarded a phantom. `onboarding_preview_mode` and
  `suggestion_prompts` stay (live empty-state seam). Behavior identical:
  constant-false branches removed. Corpus verify 0 drift; suite green.

- **iter-703 — Clean-sweep 1: live credential status fixes the stuck
  "/login to add provider" header.** Root cause of the 2026-10-09 visual-audit
  complaint 2: the jcode persistent header is auth-driven, and operant's
  adapter mapped a `has_credentials` boolean snapshotted once at `App::new`
  (auth-store + ANTHROPIC/OPENAI env only) onto the provider matrix — so any
  session that acquired credentials after boot (omp/custom base URLs,
  mid-session /login) reported NotConfigured forever and the header never
  transitioned. `credentials_live()` now recomputes per read (activation flag
  OR live auth-store OR env); `auth_status()` re-derives the provider from the
  configured model when `active_provider` is unset, and maps every
  OpenAI-compatible profile (custom-openai, omp, free-mode upstreams, groq/
  cerebras catalog) onto `openai_compatible_any`. The header gains a `custom`
  circle row for that slot when configured (unconfigured sessions — and every
  corpus golden — render byte-identically; corpus verify 0 drift). The /model
  gate, Ctrl+A model-picker gate, and `auth_method` telemetry read the live
  predicate. Tests: store-gain refreshes matrix without App recreation,
  env-isolated via AUTH_ENV_LOCK (real on-disk store cleared per test — the
  dev machine carries real credentials).

- **iter-694 — W5 Tier-2: Ctrl+L terminal-style clear (jcode parity).** The
  confirmed Tier-2 gap from JCODE-VISUAL-PARITY-PLAN.md item 17: the
  layout-side collapse was already ported (operant_ui renders a zero-height
  messages chunk when `terminal_clear_collapsed()` holds), but nothing could
  set the state. App now captures the `transcript_version` at Ctrl+L
  (`terminal_clear_version`); the state is live only while that version still
  matches and the same idle conditions jcode derives hold (not scrolled up,
  not streaming, no streaming text) — so any new output, streaming, or
  scroll-up immediately restores the full layout, no reset points to hunt.
  `clear_view_terminal_style()` snaps the scroll to the bottom; the chord
  binds next to the paste handlers. Nothing is deleted (contrast `/clear`).
  Also found while wiring: `/cls` is registered but has no execution arm —
  pre-existing, left for its owner. New corpus scenario `terminal-clear`
  (golden: cleared frame contains none of the transcript text). Label note:
  origin/main carries two iter-691-labeled commits (W5a palette sweep + the
  concurrent reaper line), and iters 692-693 (chief-of-staff gap-8 slice + its docs)
  were taken mid-prove — append-only history, all stay.

- **iter-687 — W4 animation: the original radar-pulse `signal` replaces the
  four vendored samplers.** `sample_donut`, `sample_gyroscope`, `sample_black_hole`
  and `sample_orbit_rings` (with their angle-table LUT machinery and their
  jcode-parity bit-identical tests) are deleted; the idle animation is now
  `sample_signal`, original operant code: a beam sweeps the unit disk leaving a
  rational-falloff afterglow, three staggered rings expand from the center, and
  a bright core pulses at the origin. Determinism is pinned by a
  bit-identical-across-calls test at four sizes × eight elapsed values
  (determinism replaces parity as the guarantee — the code is no longer a
  jcode port); `beam_head_leads_the_trail` and `sweeps_over_time` pin the
  physics. Exactly two transcendentals per subpixel (`sqrt` + `atan2`), no
  LUTs, vs the donut's ~142k `cos`/`sin` per frame. `IDLE_VARIANTS` becomes
  `["signal"]`, the `idle_donut_*` identifiers rename to `idle_animation_*`,
  the `three_rings`/`gyroscope` disabled-name aliases are deleted with their
  samplers, and the corpus pin `OPERANT_DISABLED_ANIMATIONS` follows the new
  name. `shape_char_3x3` and `hsv_to_rgb` stay vendored verbatim (the blit
  path is sampler-agnostic).

- **iter-686 — W3 rebrand: the vendored TUI layer stops saying jcode.** The
  seven vendored module trees rename on disk (`tui/jcode_{anim,app,markdown,
  model,render,render_core,ui}` → `tui/operant_*`), every `jcode_`/`JCODE_`
  identifier and env var with them (`JCODE_HOME` → `OPERANT_HOME`, etc.),
  and the user-visible surface with it: the onboarding header wordmark, the
  `/feedback`, `/subscription`, `/subscribe`, `/log`, `/selfdev` command
  descriptions, the login provider's display name ("Jcode Subscription" →
  "Operant Subscription"), logger stderr prefixes, two subscription-overlay
  lines, and the flicker-notice log hint. The TUI state root moves from
  `~/.jcode` to `~/.operant` (`OPERANT_HOME` override unchanged) and
  `binary_stem()` reports `operant`, so builds/logs/session markers land in
  the canonical operant state dir; existing `~/.jcode` state migrates with a
  one-time `cp -a ~/.jcode/. ~/.operant/` (leaf names are disjoint from the
  gateway's files). `AuthStatus.jcode` renames to `subscription` (serde shape
  changes only in-process). Provenance comments ("ported verbatim from jcode
  @ 0a9dc7805", "jcode parity") and the vendored file headers stay by design;
  machine-facing identifiers with state-compatibility risk (`LoginProvider
  id: "jcode"`, `SessionSource::Jcode`, `LoginProviderAuthStateKey::Jcode`)
  are deliberately kept. Corpus goldens regenerated for the rebrand; three
  goldens that had drifted with a peer's concurrent renderer edits (ask-user-
  dialog, bypass-permissions-dialog, footer-bar) ride along — absorbing peer
  drift is what a full golden regen does.

- **iter-671 — W1 of the TUI upgrade**: tool rows splice at their arrival
  anchor (`after_index` — the message count when the call started, immutable)
  instead of after their turn's last assistant message, so a tool that ran
  between two assistant messages renders between them (the "tool calls and
  final message messed up" bug); transcript turns attach tool blocks via a
  message-index→ordinal map. The legacy voice-mode surface is fully purged
  (~530 LOC): notice overlay + recorder + PTT key handling + `/voice` command
  + `tui voice` subcommand + keybinding catalogue entry (Alt+V deliberately
  uncatalogued — the registry must not advertise what nothing implements).
  Core voice/TTS and gateway `/voice` untouched.

- **iter-669 — Wave-2 guardrail harvest: ping-pong and no-progress rungs
  live on the one engine.** `ToolGuardrailTracker` absorbed the three
  patterns from the runtime's dead `loop_detector.rs` (its only consumer
  was the deleted Loop B engine; thresholds verbatim): **ping-pong** — two
  distinct tools alternating 4 complete cycles warn, 5 skip pre-execution
  (20-name sliding window); **no-progress** — a tool returning the
  identical result 5× warns, 6× arms a next-call skip regardless of
  arguments (the varied-args backstop the identical-args rung could never
  catch), fed by a new post-execution `observe_result` hook in
  `execute_tools` (synthetic guardrail skips excluded — identical by
  construction); and the **exemption port** — `with_exempt_tools`
  bypasses every repeat/loop rung and survives `reset` (the W1.8c-dropped
  `tool_call_dedup_exempt`). Skip/warn copy is pattern-aware and every
  skip variant keeps the `Guardrail: ` prefix so R35 never counts guardrail
  skips. `loop_detector.rs` deleted. Gate re-baseline: 5441 passed /
  0 failed (−15 runtime tests, +13 ported pattern tests).

- **iter-637 — one canonical required-field guard in the org stores
  (consolidation)**: `org::require_non_blank(value, message)` now owns the
  emptiness rule (`trim().is_empty()`) exactly once. The four copies
  (`authority::require_reason`, `pending_requests::require_non_blank`,
  `decisions_db::{require_mutation_reason, require_text}`) became thin
  wrappers that keep their domain message verbatim, so every error string
  the 268 org tests pin is byte-identical.

- **iter-636 — one glob matcher across every allowlist (consolidation)**:
  the two hand-rolled glob subsets are deleted and both delegate to the
  canonical `context::lcm::glob_match` — the matcher the org seat policy
  already enforced with. Three matchers meant a pattern written with a
  character class (`content.[0-9]`) matched in the org layer and silently
  never matched in the agent command allowlist and the kernel import
  allowlist. One deliberate behavior delta on kernel imports, in the
  fail-closed direction: `skills.*` now requires the literal dot (it no
  longer matches the bare name `skills`); character classes now work as
  written. Also drops `check_genome`'s unused `config` parameter (the
  genome checks read the org DB directly, not the config).

- **W1.8c — channel dispatch runs on the reconciled facade** (the last
  `run_tool_call_loop` call site): `process_channel_message` now constructs a
  `ReconciledAgent` per message via `FacadeConstruction` (provider route,
  system prompt, `temperature`, the channel's `max_tool_iterations`, the
  Loop-B-parity tool allowlist with non-CLI exclusions and the pacing
  `loop_detection_enabled` switch), pins a per-conversation core session id
  (re-minted on `/new` and on rollback), hydrates the fresh session transcript
  from the orchestrator's sanitized prior turns, and runs the turn through
  `run_facade_turn` with the old loop's scope chain at the call site
  (`Box::pin` keeps the large turn future off the polling stack — a 2MB
  tokio worker overflowed inside `Regex::new` in context-reference
  preprocessing before this). Cross-crate fixes the migration surfaced:
  (1) prompt-mode providers now receive tool results as user-role
  `[Tool results]` text (`operant-api` `Provider::chat` default) — core's
  role-`tool` JSON was invisible to text-only providers and re-triggered
  calls until the loop detector tripped; (2) the facade's session store
  enables `PRAGMA busy_timeout=500` and `BEGIN IMMEDIATE` on its write
  transactions — concurrent facades (channel turns, cron, sub-agents) died
  instantly with `SQLITE_BUSY_SNAPSHOT` ("database is locked"); (3) the
  guardrail duplicate-skip result is success-shaped guidance (`Guardrail: …`,
  no `[` prefix — the anomaly heuristic treated a bracket prefix as failed
  JSON and refunded iterations re-asking the model); (4) channels substitute
  core's empty degenerate grace summaries with a stopped-early notice naming
  the configured iteration budget, and strip raw tool-call/result JSON
  artifact lines from final replies; (5) test facades point at per-context
  temp data dirs instead of the operator's live `~/.operant` (39 contexts
  contended on one SQLite file). Channels suite: 1045/0.

- `org grant give` / `org grant revoke` (F1, ORGANISM-ARCHITECTURE §6): the
  CLI no longer writes the `authority_grants` ledger directly — every mint
  and revocation goes through the `SeatApprover`, the same grantor-scope
  ceiling, TTL shape, and vacant/unmanaged-seat fail-closed the gateway's
  `/grant` and `/revoke` get. `GrantDb::insert`/`GrantDb::revoke` are now
  `pub(crate)`: outside `operant-core` the approver is the only write path,
  enforced by the compiler rather than by convention. The flags keep their
  shape but become assertions about the approver-derived grant —
  `--scope` must equal the derived scope, `--target-dept` (when given) the
  derived pin, and `--expires-at` is honored as a whole-day TTL override
  (mirroring `/grant`'s `--days`); a request the approver would not mint is
  refused before any row is written, with the refusal naming why.

### Removed

- **iter-797 — `tui::transcript_turn`** (`build_transcript_turns`, `reasoning_heading`):
  a second transcript model with no production consumer; the adapter is
  the single path. Its two grouping tests went with it.

- **iter-664 — the dead channels dispatch orchestrator (audit-r2 plan §2,
  Option A)**: `start_channels` (the only pub entry), the dispatch chain
  (`run_message_dispatch_loop` → `dispatch_worker` →
  `process_channel_message`), and every sibling consumed only by that chain —
  `commands`, `consts`, `dispatch`, `factory`, `health`, `history`,
  `identity`, `memory_ctx`, `media_pipeline`, `prompts`, `routing`,
  `runtime_types`, `sanitize`, `supervision`, `startup`, and the
  orchestrator-level `tests.rs` (~167 tests) — plus the dead
  `orchestrator::deliver_announcement` registry variant and
  `ChannelNotifyObserver`. Audit trail: the shipped CLI gateway never
  called any of it (live paths are `gateway_runner.rs` + `operant-gateway`;
  LTO stripped the cluster from every deployed binary since iter-628), the
  only external `channels::orchestrator::` consumer is `acp_server`, and no
  production caller of `register_delivery_fn` exists. The one live helper
  the cluster exported, `strip_tool_call_tags`, moved verbatim to
  `telegram::helpers`. `acp_server` and the feature-gated `mqtt` listener
  remain. Five-crate gate re-baselined: 5443 passed / 0 failed across 44
  binaries.

- **W1.10 — Loop B engine deleted**: `run_tool_call_loop`
  (`loop_/tool_loop.rs`), the `agent_turn` legacy wrapper (`loop_/turn.rs`),
  and their 35 unit tests, fixtures, and the five `*_loop_c`
  direct-drive parity tests go away with zero remaining production callers
  (channel dispatch has run on the reconciled facade since iter-662;
  `loop_detector.rs` was already harvested into `core::tool_guardrails` in
  Wave 1). `agent_parity` keeps the core↔Loop C pairs (10 tests).
  `ChannelRuntimeContext` drops the fields only the engine consumed —
  `multimodal`, `tool_call_dedup_exempt`, `activated_tools` (the
  context copy; the tool-registry's own `ActivatedToolSet` handle stays),
  and `max_tool_result_chars` — from the struct, startup wiring, and all 39
  test contexts. Re-baseline: runtime lib 1654/0, parity 10/10; five-crate
  gate 5610 passed / 0 failed across 44 binaries.

- `operant-runtime::doctor` (1,347 lines) — the duplicate engine. Only the
  gateway's `GET /api/doctor` consumed it; its uncalled `run`/`run_models`/
  `run_traces` entry points went with it. Its checks live on in
  `operant-core::doctor`, adapted to `AppConfig` where possible. Named
  gaps (fields only `schema::Config` carries, or state nothing writes —
  porting would be faking it): channels-configured, delegate-agent
  provider validity, `gateway.port`, `memory.embedding_model` hint
  targets, config-file presence (`config_path`), the runtime
  `workspace_dir` and its SOUL.md/AGENTS.md presence checks (the AppConfig
  world's data root is `operant_home()`; the workspace concept did not
  carry over), and daemon heartbeat freshness (`daemon_state.json` is
  written only by `operant-runtime::daemon::run`, which has no callers).
- `[genome].queued_cron_jobs_resolve_grants` — declared in wave 2 but the
  scheduler sweep it described never gained a consumer (iter-600). Pending
  escalations resolve by a human verdict or the 60s interactive lapse only.
  Breaking for configs that set it (the key shipped commented; if you
  uncommented it, delete the line): `[genome]` keeps `deny_unknown_fields`
  because a silence-tolerant typo on a security knob failing open to the
  `yolo` seating default is the worse failure mode.

The permission genome — per-employee authority for the organism (iters
573–586, wave fleet execution). A seat's policy is data (`seat_policies`:
mode yolo/standard/scoped/lockdown + allow/deny globs), the precedence is a
pure function (`decide()`: blocklist absolute, seat-deny absolute, allow,
yolo, standing grant, then mode defaults), and the run path consults it
behind an Option so ungoverned seats are byte-identical to before.
Escalations persist to a durable `pending_requests` queue; unattended
(cron) runs enqueue and deny the same run, and a senior's approval mints a
TTL'd grant via the existing grant ledger so the next tick runs without
re-asking. Reporting lines are data (`hierarchy_edges`). The scheduled-run
write barrier is live: every completed cron run leaves exactly one
attributable worklog row, and a barrier write failure fails the run.
`[genome]` config block lands with `grant_ttl_days = 7` default.
Also in this wave: BUGS.md D-5 (two MemoryManagers over one MEMORY.md)
fixed by hoisting one `(MemoryManager, provider)` pair for both agents;
D-2 and D-3 resolved (see BUGS.md); K-2 filed (pre-existing order-dependent
`loop_request_timeout` pair).

## [0.2.1] - 2026-09-30

Foundation for post-turn features (reflection / advisor / dreaming).
A `TurnEnd` event (turn id, iterations, tool counts/durations, capped result
summary) is emitted once per completed turn at the turn-end chokepoint onto a
`TurnEndBus` broadcast seam; with no subscribers attached the emit site costs
a field read. Deterministic tool-loop anomaly detectors (empty result,
malformed output, error signal) re-enter the loop through the existing
continuation shape, capped at 3 retries per turn; the dead `evaluate_response`
self-critique now executes on the response path with its score exposed.
Batches with >1 file mutation or any approval-gated call run sequentially
instead of sharing the 8-worker pool; all provider stream tasks are bound to
their stream handle (`AbortOnDrop`) so early drops stop the task.

Parity work against the `jcode` reference agent (iters 347-409). The TUI
gained roughly 3,000 lines; 1,146 lines of long-dead code were deleted.

### Changed

- **iter-716 — Clean-sweep 6: integration-truth fixes — `tool_backend` stops
  mis-advertising the live web_search chain.** The introspection tool claimed
  `tavily, exa, searxng, ddg` with `tavily` current — but the live chain is
  `sourcehound → tavily → exa → ddg → searxng` with `sourcehound` the config
  default and DDG the key-free fallback. Map + description now name sourcehound
  and default to it (static claim of the config default, noted in source).
  Stale `igs`-alias comment in web_tools.rs corrected (the config default is
  `"sourcehound"`, not "the old string"); the alias itself stays — pre-re-home
  configs depend on it reaching the key-free engine. NOT deleted (audit
  candidates re-verified as live): operant-tools `WebSearchTool` + routing feed
  the runtime agent registry consumed by the ACP orchestrator; the obscura/
  lightpanda names in browser_provider are intentional retired-name fallbacks
  with a pinned warning test. Suite: tool_backend 3/3 on clean origin/main
  (the shared tree is red from a peer in-flight core change — collect_map_nodes
  — unrelated to this delta).

- **iter-714 — Clean-sweep 5: composer text-area selection (jcode parity).**
  Shift+Left/Right select characters in the composer; Shift+Up/Down extend
  across visual rows (vim Visual mode keeps its existing Shift+arrow path);
  the selection renders reverse-video through the vendored wrap renderer
  (ui_input::wrap_input_text splits each wrapped segment into before/selected/
  after spans); Ctrl+C copies the composer selection and suppresses exit-confirm
  while one is live; any other edit key collapses it. The vendored surface
  gains `TuiState::input_selection()` (default None — no other implementor
  affected). Multibyte-safe via char-boundary clamping. Suite 1745/0 incl.
  3 selection tests (chords, backwards anchor, reverse-video render with
  surrounding chars intact).

- **iter-710 — Clean-sweep 4: Ctrl+Up/Ctrl+Down fine transcript scroll
  (jcode parity).** Completes the keyboard scroll ladder: ±3 on Ctrl+arrows,
  ±10 on PageUp/PageDown, ±20 on Alt+arrows, mouse wheel otherwise. Without
  the fine rung a mouseless session (tmux without mouse-mode, plain tty —
  wheel events arrive as nothing) could only jump in 10-line quanta; part of
  the "chat window is pinned" complaint (2026-10-09 visual audit). Both
  chords pause tail-follow; reaching the bottom resumes it. Suite 1742/0
  incl. chord-step regression test.

- **iter-709 — Clean-sweep 3: `last_msg_area` republished — the dead right-click
  context menu lives again.** The iter-648 chrome cutover deleted the
  dispatch-row writer that fed `last_msg_area`; every consumer (right-click
  message menu bounds, Ctrl+Shift+M cursor-open menu, menu clamping) gated on
  a zero rect and could never fire — production-dead since the cutover,
  flagged by the 2026-10-09 visual audit. The render pass now publishes it
  from the vendored `message_area()` alongside the other three hit-test cells
  (same post-draw discipline as the iter-661 fix). Pinned by a default-zero +
  setter-semantics unit test. Corpus verify 0 drift; suite 1741/0.

- **Memory backend is now memory-wire (in-process).** `MemoryWireProvider`
  calls the `memory_wire` crate's sync `retain`/`recall` API directly with a
  `catch_unwind` boundary (a panic degrades to a memory miss, not a dead
  turn). `provider = "memory-wire"` is the default; `"agentmemory"` still
  resolves to it. Upstream tracks branch `main`; advance with
  `scripts/sync-vendors.sh`, pin `rev =` at release.
- **Web + browser backends are now sourcehound (MCP subprocess).**
  `web_search`/`web_scrape`/`web_extract`/`web_crawl` and the `sourcehound`
  browser provider (`cloakctl.navigate`/`read`/`act`, raw CDP via
  `cloakctl.cdp`) run over the `sourcehound` binary's stdio MCP server.
  `browser.provider` and `preferred_provider` default to `"sourcehound"`.
  In-process was measured unviable (their `[patch.crates-io]` evaporates for
  consumers; unconditional render stack).

### Removed

- **agentmemory integration deleted.** `AgentMemoryProvider`, the `:3111`
  `npx` auto-spawn, and the deferred agentmemory MCP registration are gone.
  `memory.agentmemory_*` config keys no longer exist; old configs carrying
  them fail parse under `deny_unknown_fields`.
- **IGS + Obscura implementations deleted.** `tools/igs.rs`, the IGS binary
  surface, `ObscuraProvider`, and `obscura_cdp.rs` (renamed to
  `sourcehound_cdp.rs`, re-homed onto `cloakctl.cdp`) are gone. `tools.*`
  and `browser.provider` values `igs`/`obscura` no longer exist; the retired
  names fall back to lightpanda (browser) and sourcehound (search).

### Fixed

- **`/steer` was a guaranteed no-op.** Submitting it required `Enter` while not
  streaming, but the queue path rejected exactly that state — the two halves
  were mutually exclusive, so the command had never worked despite being
  documented. The core loop's steering was fully implemented and unreachable.
- **A compaction cut could orphan a `tool_use`/`tool_result` pair.** Eviction
  keeps a head and a recency tail, so a cut could land between a request and its
  answer; providers reject either half alone, and an unanswered request leaves
  the model waiting so it reissues the call and burns a turn. A shape-independent
  repair now runs at the single chokepoint both eviction paths pass through.
- **`/keys` advertised two chords that never worked.** `Ctrl+H`/`Ctrl+L`
  ("previous/next history") had no dispatch arm anywhere, and cannot be bound
  anyway — `Ctrl+h` is ASCII 8 (backspace) and `Ctrl+l` is readline's
  clear-screen. The claims were removed and both chords added to the terminal
  conflict list that `/hotkeys` uses to explain unavailability.
- **`operant-core`'s `simple_agent` example did not compile** (a config field
  added in iter-357 was missing from its struct literal). Note that
  `cargo check --workspace` does NOT catch this — only `cargo test --workspace`
  builds example targets.
- **`operant chat` never exited on a non-TTY stdin.** `read_line` reports a byte
  count and `0` means EOF, but the loop discarded it, so EOF was indistinguishable
  from a blank line and the `continue` spun forever — 33 MB of `You: ` in 20 s
  with stdin on `/dev/null`. `operant run --query` handled the same condition
  correctly, so the fallback existed and was simply not wired to `chat`/`tui`.
- **`operant <cmd> | head` aborted with a core dump.** Rust's runtime sets
  `SIGPIPE` to `SIG_IGN` before `main`, so the first write to a closed pipe
  returns `EPIPE`, which `println!` treats as a broken invariant and panics on;
  the release profile's `panic = "abort"` then turns that into SIGABRT plus a
  core file. Restoring the default disposition makes a closed pipe behave like
  every other Unix tool (exit 141, silent), matching `yes | head -1`.
- **`embedded-web` embedded nothing, on any machine, ever.** The build script and
  the `include_dir!` both pointed at `<repo-root>/web/dist`, a path that has never
  existed in this layout; the Vite project writes to `crates/operant-cli/src/dashboard`.
  Because the build script only sets its cfg when the dist exists, the `include_dir!`
  was never even compiled, so the feature silently degraded to the filesystem
  fallback while passing `--all-features`. A feature that reads as wired and
  reaches nothing.
- **`/terminal-setup` was a stub that hid the problem.** It printed "No manual
  setup needed" while a real interoperability gap sat behind it: without
  kitty-protocol parsing, most terminals cannot send a distinguishable Shift+Enter.
  The command now reports the terminal's actual capabilities.
- **vim `j` and `k` were advertised and dead.** `/keys` listed `VimMotionDown`/
  `VimMotionUp` for the two most fundamental vertical motions; `vim_normal` had
  no arm for either, the `apply_vim_command` referenced in a comment did not
  exist, and every `motion_*` helper in the file was horizontal.
- **Sixel was claimed from a multiplexer's `$TERM`.** Detection inferred
  graphics capability from tmux's own terminfo rather than the outer terminal, so
  it was wrong in both directions — the common `$TERM` never matched, and
  `tmux-256color` claimed Sixel unconditionally. Pinned graphics re-emit every
  frame, so a wrong claim turned a one-shot glitch into a permanently mangled
  image.
- **`operant doctor` always exited 0**, printing "Found 4 issue(s) to address" and
  then reporting success, so `operant doctor && operant chat` walked straight
  into the failure doctor had just described. A diagnostic that cannot fail is a
  diagnostic nobody can gate on.
- **The default theme's accent leaked past the palette in 14 places.** Banner,
  prompt input, spinner, model-picker highlight and four dialogs hardcoded
  `Rgb(255,191,0)` — the *default* theme's accent — so all 7 other themes showed
  them amber.
- **Four `/keys` entries named the right chord with the wrong action.** The chord
  was dispatched, just not as advertised: Ctrl+A was catalogued "move to start" but
  opened the model picker, Ctrl+B "word left" opened the session branch browser,
  and Ctrl+P was wrong twice. A wrong-action key is worse than a dead one — a dead
  key is visibly dead.
- **Three advertised prompt bindings did nothing** (Ctrl+E, Ctrl+F, Ctrl+N).
  `/keys` listed them, the dispatcher had no arms.
- **A test double shipped in the production library.** `VirtualClock` was `pub`
  with no `#[cfg(test)]`, dragging `Mutex::lock().expect()` in with it.
- **The build scripts only worked on one machine.** `dev-env.sh` and
  `provision-build-deps.sh` hardcoded `/home/z/my-project/local`, so on any other
  host every export resolved to a nonexistent directory. Both now derive every
  path from one `LOCAL_DIR` defaulting to the repo's own `local/` — which is where
  `operant-core/build.rs` already searched. `install.sh` now preflights and names
  the missing package instead of failing 200 lines into a linker error.
- **A dangling symlink was committed.** `provision-build-deps.sh` created
  `local/lib/libsonic.so` with an unconditional `ln -sf`, which succeeds even when
  the target is absent; the resulting dead link was then committed, past
  `.gitignore`. It was never load-bearing — the build resolves from the committed
  `libsonic.a` alone.
- **The first error a new user saw named no remedy.** `operant run --query` on a
  fresh machine printed an internal-sounding pool error with no hint that
  `operant setup` was the fix. `operant doctor` got this right; the path a new user
  actually takes did not.
- **`operant doctor` advised a command that cannot work.** It probed a
  `tinker-atropos` submodule that exists in neither the repository nor
  `.gitmodules`, and recommended `git submodule update --init --recursive`.
- **A turn could never time out.** `reset_on_success()` was called from the `Ok`
  arm of the response match — once per agent *iteration*, not once per turn — so
  a turn alternating "stream dropped" with "answered fine" had its retry budget
  refilled between failures and never exhausted it. A flaky upstream held a
  gateway turn open for 9 minutes against a 120s per-request ceiling that bounds
  a single call, not a turn. A monotonic per-turn `turn_retry_failures` counter
  (cap 12, deliberately not the per-call `max_retries` default of 3) and a
  20-minute wall-clock limit now terminate such a turn; the per-call budget
  still refills per iteration.
- **Model output that can name credentials was logged.** The tool-call parser
  logged 80-100 chars of model-emitted text verbatim on both the parse-failure
  and repair paths, and that text is routinely shell fragments — so shell
  fragments, which name credentials, reached the server log. Worse, the
  parser's `ParserEvent::Error` was injected into the *conversation*, handing the
  raw fragment back to the model and persisting it in session history. The
  parser now logs `content_len` plus a preview cut before the first `$`, `=`, or
  quote; the parser error is a fixed re-issue sentence with no model output; and
  the safety layer logs lengths only.
- **A turn lost to a restart was detected and then silently discarded.**
  `check_interrupted_turns()` logged a WARN naming the channel and returned
  unit, so the caller threw the `(channel_id, timestamp)` pairs away: a run that
  started, died 5.2s later, and rebooted showed the user 18 minutes of silence
  with no notice. The boot path now delivers the notice through
  `send_channel_message`, and the state file records the platform so a
  non-Telegram turn is not announced on Telegram (pre-existing files without the
  field fall back to telegram, the only adapter that recorded turn state).
- **Channel feature flags selected nothing.** The gateway's dependency line
hardcoded 21 `channel-*` features, so cargo's feature union switched them all
on no matter what was requested — dropping channels from the CLI defaults
produced a byte-identical binary. Each CLI `channel-*` now forwards to both
`operant-channels` and `operant-gateway`, the hardcoded list is down to the
9 the gateway's own source structurally requires, and a manifest-contract test
locks the wiring. Default builds resolve the same 23 channels as before.
- **The prompt-injection detector ran nowhere.** `PromptGuard` (override /
role-confusion / JSON-injection / exfiltration / jailbreak patterns) compiled
and re-exported with zero production call sites. It now scans inbound text at
the runtime turn entry point and records the verdict; the default stays
`Warn`, so turns proceed exactly as before.
- **Five `web_search_tool` tests flaked on a leaked proxy, not on wiremock.**
`proxy_config` tests mutated the process-global proxy without restoring it,
and the mock-backed search tests read it back — routing loopback URLs to a
dead port. A snapshot-restore guard plus a shared reader/mutator mutex fixed
it (0 failures in 13 parallel runs after 4-of-7 failing).
- **`lifeos_enabled` configured nothing and is gone.** The key gated no code —
no LifeOS tool module or cargo feature exists — so it was removed from the
schema and the example together. Existing configs carrying it now fail parse
under `deny_unknown_fields` instead of being silently ignored.

### Added

- **Turn lifecycle state machine** — a 9-state `TurnState` (idle, sending,
  connecting, thinking, streaming, running-tool, awaiting-approval,
  waiting-for-network, compacting) replaces the single `is_streaming` bool for
  display, plus per-turn wall-clock, tokens/sec, and input/output/cache deltas
  in the footer.
- **Tool-call grouping** — concurrent calls group with an `N/M done` meter and
  per-sub-call status, and a call waiting on the tool-pool semaphore now
  renders as queued rather than running.
- **Real markdown rendering** — `pulldown-cmark` drives lists (nested, ordered,
  task), italic, strikethrough, horizontal rules and footnotes. Previously
  `- item` printed literally and the renderer claimed italic support it did not
  have.
- **One clipboard path** with a five-mechanism fallback chain
  (arboard → wl-copy → xclip → xsel → OSC 52). Writes previously failed
  silently over plain SSH. Drag-select copy mode added.
- **Eight live themes** including a deuteranopia-safe palette, `reduce_motion`
  now honoured by the animation path, and Kitty/iTerm2/Sixel image rendering
  wired to paste.
- **Truecolor detection** with perceptual xterm-256 quantization, and
  grapheme-cluster + cell-width measurement (a CJK ideograph is one cluster but
  two cells, so cluster counting alone is still wrong).
- **Mermaid diagrams** render as images through the existing image stack, with
  text and raw-source fallbacks, on a background worker.
- **Provider factory registry** — a provider is now one trait impl plus one
  line, with a structural `RuntimeKey` (not a display string) and a typed
  `FailoverDecision` distinguishing a rate limit from a broken model.
- **OAuth PKCE + device-code login** with an account pool that stores only keys
  and order (never tokens) and a per-account single-flight refresh coordinator,
  so N concurrent requests cannot each spend the same single-use rotating
  refresh token.
- **Destructive-command gate** hooked into the existing approval path —
  structural blast-radius classification, a justification gate that measures
  substance rather than length, and system-path denial. Defense-in-depth, not a
  sandbox.
- **Cache-miss detection** — a prefix tracker over the stable prefix (system
  prompt + tools + skills), with proven and inferred verdicts reported
  separately. A detector reporting a hit it cannot know is worse than none.
- **Tool-name recovery** — a hallucinated or near-miss tool name is recovered by
  bounded Levenshtein, and the model is told the recovery happened. It refuses
  on an equidistant tie, because arbitrarily choosing is how you get a
  confidently wrong action.
- **Swarm workers** — real headless workers running their own agent loop, with a
  spawn guard that checks depth and a *global* breadth cap before spawning and
  releases its count on every exit path.
- **Keybinding registry wired** — `/keys` and `/hotkeys` read the catalogue and
  count real usage; input stash (`Ctrl+S`), burst undo (`Ctrl+Z`) and queued
  message recall.

- **A deployability plan** (`docs/DEPLOYABILITY-PLAN.md`) recording, from verified
  evidence rather than inference, what actually blocks shipping: two crashes on
  non-interactive paths, a native build chain that only resolved on one machine,
  a release that has never been cut, and a `main` branch that had been continuously
  red. The headline is that feature work is essentially done and deployability is
  not — nothing is wrong with what is *in* the tree; everything around it was.
- **A usage overlay** (F8) and **background task rows** showing live delegation
  state, both fed by data that already existed but was never surfaced.
- **`/terminal-setup` reports real terminal capabilities** instead of claiming
  there is nothing to do.
- **Behavioural keybinding tests** across the prompt, global and vim contexts, and
  a tokenizer-based reachability pin covering all 69 vim bindings.

### Changed

- **iter-716 — Clean-sweep 6: integration-truth fixes — `tool_backend` stops
  mis-advertising the live web_search chain.** The introspection tool claimed
  `tavily, exa, searxng, ddg` with `tavily` current — but the live chain is
  `sourcehound → tavily → exa → ddg → searxng` with `sourcehound` the config
  default and DDG the key-free fallback. Map + description now name sourcehound
  and default to it (static claim of the config default, noted in source).
  Stale `igs`-alias comment in web_tools.rs corrected (the config default is
  `"sourcehound"`, not "the old string"); the alias itself stays — pre-re-home
  configs depend on it reaching the key-free engine. NOT deleted (audit
  candidates re-verified as live): operant-tools `WebSearchTool` + routing feed
  the runtime agent registry consumed by the ACP orchestrator; the obscura/
  lightpanda names in browser_provider are intentional retired-name fallbacks
  with a pinned warning test. Suite: tool_backend 3/3 on clean origin/main
  (the shared tree is red from a peer in-flight core change — collect_map_nodes
  — unrelated to this delta).

- **iter-714 — Clean-sweep 5: composer text-area selection (jcode parity).**
  Shift+Left/Right select characters in the composer; Shift+Up/Down extend
  across visual rows (vim Visual mode keeps its existing Shift+arrow path);
  the selection renders reverse-video through the vendored wrap renderer
  (ui_input::wrap_input_text splits each wrapped segment into before/selected/
  after spans); Ctrl+C copies the composer selection and suppresses exit-confirm
  while one is live; any other edit key collapses it. The vendored surface
  gains `TuiState::input_selection()` (default None — no other implementor
  affected). Multibyte-safe via char-boundary clamping. Suite 1745/0 incl.
  3 selection tests (chords, backwards anchor, reverse-video render with
  surrounding chars intact).

- **iter-710 — Clean-sweep 4: Ctrl+Up/Ctrl+Down fine transcript scroll
  (jcode parity).** Completes the keyboard scroll ladder: ±3 on Ctrl+arrows,
  ±10 on PageUp/PageDown, ±20 on Alt+arrows, mouse wheel otherwise. Without
  the fine rung a mouseless session (tmux without mouse-mode, plain tty —
  wheel events arrive as nothing) could only jump in 10-line quanta; part of
  the "chat window is pinned" complaint (2026-10-09 visual audit). Both
  chords pause tail-follow; reaching the bottom resumes it. Suite 1742/0
  incl. chord-step regression test.

- **iter-709 — Clean-sweep 3: `last_msg_area` republished — the dead right-click
  context menu lives again.** The iter-648 chrome cutover deleted the
  dispatch-row writer that fed `last_msg_area`; every consumer (right-click
  message menu bounds, Ctrl+Shift+M cursor-open menu, menu clamping) gated on
  a zero rect and could never fire — production-dead since the cutover,
  flagged by the 2026-10-09 visual audit. The render pass now publishes it
  from the vendored `message_area()` alongside the other three hit-test cells
  (same post-draw discipline as the iter-661 fix). Pinned by a default-zero +
  setter-semantics unit test. Corpus verify 0 drift; suite 1741/0.

- **`operant acp` gained the real protocol surface** — `initialize`, `session/new`,
  `session/prompt`, `session/cancel`, and streaming `session/update` notifications.
  The four original methods are gone; the envelope validation (including the
  notification-vs-explicit-null-`id` distinction) is kept.
- **Pinned graphics survive redraw.** Pasted images and Mermaid rasters were
  written to stdout once and forgotten, so both vanished on the first scroll — any
  redraw destroyed them permanently. They are now laid out in the frame and
  re-emitted after ratatui's flush, positioned with an explicit cursor move
  (Kitty escape sequences carry no coordinates), reserved in the cell grid so they
  cannot overlap the usage overlay or task rows.
- **LaTeX fences render** via a background worker (`dvipng` when present, a
  Unicode typesetter otherwise), so the render loop never blocks on a subprocess.
  Pending results report as pending rather than appearing blank.
- **The release pipeline can publish.** `release.yml` gates on the whole Build
  workflow's conclusion, so one unbuildable cross target (Android needs an NDK,
  FreeBSD needs a sysroot) silently blocked every release while the native
  binaries sat complete and unpublishable. The cross job is now explicitly
  best-effort; the native job still gates, because those three artifacts *are*
  the release.
- **The release build no longer depends on a third-party submodule.**
  `submodules: recursive` meant every release required `vendor/prime-agent` to be
  reachable, for a `[pk]` component that ships `enabled = false`. Verified
  (`cargo build --workspace` completes with the submodule uninitialised) rather
  than assumed. `test.yml` still initialises it — the kernel tests genuinely need
  it at run time.
- **284 foreground colour sites moved onto the theme palette** (accessor calls
  236 → 530), so the bulk of the named `Color::*` variants are themed rather than
  hardcoded. Five sites are intentionally left alone where the colour encodes a
  syntax or tool-category role the palette does not model.
- **Keybinding drift is guarded behaviourally, not just structurally.** Binding
  catalogues are now checked by pressing the chord and asserting the resulting
  state, because a source-level grep passes for a binding that is dispatched to
  the *wrong action* — which is the failure mode that actually bit `/keys`.
- **Oversized tool results are withheld, never silently truncated.** A result
  that would exceed `agent.max_tool_result_share` (default 5% of the context
  window) is replaced by a marker stating the kept head, the token count, the
  per-token price and the tokens removed.
- **1146 net lines of dead TUI deleted** — `voice_capture.rs`, `state.rs` and
  `terminal.rs` removed outright, `messages/cache.rs` superseded. Three sibling
  files had carried `#![allow(dead_code)] // wired in Phase 2I` with zero
  consumers.

### Known limitations

- **MCP turn-triggered materialisation is net-negative on short turns.** On a
  realistic 40-tool catalog a 4-token generic turn materialises 24 of them, so
  it *increases* the visible schema. Kept as a convenience (a relevant tool
  without a discovery turn), not a token optimisation; the number is pinned in
  a test.
- **Hybrid retrieval scoring is unproven and deliberately unwired.** Measured
  recall got *worse* (R@3 2/5 → 1/5). A mutation-proven gate fails the build if
  anything outside `retrieval.rs` calls it.
- **Themes still do not reach the whole TUI.** iter-414 migrated every
  foreground `Color::{White,Yellow,Cyan,Red,Green}` to a palette accessor
  (accessor calls 236 → 530), and later work removed the accent leak that kept
  14 sites amber on every non-default theme. Still unmigrated: ~89 explicit
  `Color::Rgb`/`Color::Indexed` literals (~80 distinct values), plus `DarkGray`
  and `Black` sites. These are deliberately NOT queued as a mechanical sweep:
  `DarkGray`/`Black` are fixed ANSI palette slots and therefore theme-invariant by
  construction, the default theme's `muted()` is a dim *amber*, and the transcript
  uses a deliberate cool-grey scheme — so a name-based mapping would repaint the
  UI. Each remaining site is a per-theme appearance decision, not a refactor.
- **`operant acp` implements only part of the Agent Client Protocol.** It now
  handles `initialize`, `session/new`, `session/prompt` and `session/cancel`,
  and streams `session/update` notifications with content blocks — previously it
  was a hand-rolled JSON-RPC with four unrelated methods that shared only a name
  with ACP. Dispatch was widened (`DispatchOutcome` + a `SessionRegistry`) because
  ACP's methods carry session state and `session/update` is a notification, which
  a single `RpcResponse` return could not express. Still missing versus the real
  protocol: `session/load`, `authenticate`, and a proper permission
  request/response round-trip.
- **There is no SDK server and no harness-api-server.** The `operant-harness`
  crate exposes no API surface; `operant status --json` and
  `architecture dump --live` are the only machine-readable surfaces today.


## [0.2.0] - 2026-09-27

### Security

- **`code_execution` and `patch` ignored the approval blocklist.** The command
  extractor read `args["command"]` for tools whose arguments are `code`/`path`,
  so the payload fell through to the literal tool name and matched nothing.
  `code_execution` runs unsandboxed, which made the approval blocklist its last
  real mitigation — and it was inert. Both tools now have their own extraction
  arm.
- **Credential-pool keys were written to `config.toml` in plaintext.** The
  primary `api_key` was encrypted; the pool's `api_keys` vector was not.
- **The full parent environment — including every API key — reached every
  spawned process.** The terminal backend passed `std::env::vars()` through, and
  an empty `env_vars` inherited everything implicitly. A secret-pattern scrub now
  runs at all six spawn sites. Deliberately a denylist, not an allowlist: an
  allowlist would strip `PATH`/`HOME` and break arbitrary user commands.

### Added

- **Lossless Context Management (LCM) engine** (`agent.context_engine = "lcm"`) — hermes-lcm parity: an append-only SQLite DAG keeps every message verbatim (FTS5-indexed) while the fresh-tail (D0) window stays in context. Opt-in; the built-in `compact` engine remains the default.
- **LCM rollups (P1)** — on-demand day/week/month LLM summaries (`operant context rollup`), stored idempotently in `lcm_rollups`; over-budget contexts inject stored rollups into compaction (`context_lcm_rollups_inject`, default on) so the model reads condensed history while the DAG stays lossless.
- **LCM maintenance** — on-demand sweep (`operant context rollup-maintenance`) and a config-gated background scheduler (`context_lcm_rollup_interval_minutes`, 0 = off) that build missing rollups for all sessions; a bad LLM pass is logged and never aborts.
- **LCM agent tools (P2)** — `lcm_recall` (verbatim FTS recall), `lcm_stats` (engine diagnostics), `lcm_assert` (durable, conflict-preserving fact store with active-state resolution and contradiction reporting), and `lcm_recall_round` (multi-round evidence-gated retrieval with cumulative exact evidence and search leads).
- **LCM adaptive auto-recall (P3)** — one bounded retrieval round per assemble against the latest user message injects a system "pre-answer evidence" block (`context_lcm_auto_recall`, default on).
- **AFT tool bridge** — optional native integration that surfaces the AFT code-toolkit (`aft_read`/`aft_write`/etc.) to the agent when the `aft` binary is available, with natural fallback to operant-native tools.
- **Harness kernel surfaces** — `operant status --json` exposes `harness.metrics` (mount ok/pending/failed counters, best-effort boot) and `architecture dump --live` attaches the same `HarnessMetrics` block, so operators can see mount outcomes and alert on PENDING churn.
- **Configurable prompt-cache TTL** — `client.prompt_cache_ttl = "5m" | "1h"` (OpenRouter path honors it; Anthropic clients mark breakpoints with the ttl field).
- **WhatsApp markdown** — outbound WhatsApp messages optionally convert `*bold*`/`_italic_`/`~strike~`/fences when `parse_markdown` is set (shared `asterisk_dialect` with Slack mrkdwn).
- **Pinned agentmemory spawns** — every runtime `npx @agentmemory/*` spawn (auto-spawn + MCP entries) uses the pinned `DEFAULT_AGENTMEMORY_VERSION` (0.9.29), overridable via `[memory] agentmemory_version` / `AGENTMEMORY_VERSION`. A cold boot no longer fetches whatever npm serves at that moment.
- **Mount-cap enforcement** — `max_active_providers` is enforced per mount (config default 64) with a cheap `provider_count()` check, preventing OOM from runaway mounts at large pool counts.

### Fixed

- **Piping any long-output command aborted with a core dump** (SIGPIPE
  disposition). `operant doctor | head` now exits 0.
- **`operant chat` looped forever on a non-TTY stdin**; EOF is now terminal.
- **`operant doctor` always exited 0**, printing failures and reporting success,
  so nothing could gate on it. It can now fail — and advisories ("install git",
  "not logged in") are explicitly *not* counted as failures, so a healthy install
  with unconfigured optional integrations still exits 0.
- **The build could not succeed on a machine that had never built operant.**
  `dev-env.sh` pointed at another developer's home directory and died sourcing a
  `~/.cargo/env` that does not exist outside rustup installs; the dependency
  provisioner was Debian-only and failed outright without `dpkg-deb`.
  Both now detect what the host actually has.
- **`/keys` advertised four chords that did something else**, and three prompt
  bindings that were documented but never provided.
- **web_search per-provider timeout** — every candidate engine is now bounded by `search_timeout_secs` (`run_provider_chain`), so a hung provider (e.g. a stalled igs subprocess whose own timeout can reach 60s) fails over to the next engine instead of killing the whole search at the agent-loop tool timeout. Timeouts/errors/empty results all fall through and the timeout is surfaced in the error.
- **agentmemory default alignment** — schema `MemoryConfig::default()` backend was `sqlite` while the core AppConfig default, docs, and `operant.example.toml` said `agentmemory`; the schema default is now `agentmemory` so the daemon/gateway path matches the CLI path (the injected MCP server stays deferred, and `ensure_backend` degrades gracefully with a warning when Node.js/npx is unavailable).
- **memory tool guidance** — `memory_store`/`memory_search`/`memory_recall` descriptions now state they target the builtin MEMORY.md store and point to `memory_save`/`memory_smart_search` for the agentmemory backend, so agents route to the correct memory surface.
- **`OPERANT_CONFIG_DIR` honored by every load path** — `default_config_paths()` (operant-core) now resolves `<OPERANT_CONFIG_DIR>/operant.toml` before the XDG/`~/.operant` fallbacks, matching the schema layer. Previously `operant run` (and the `config_manage` tool) silently ignored the env var and loaded the real `~/.operant/operant.toml`, so supposedly-isolated runs could mutate the user's real config.
- **Agentic self-management guidance** — `SKILLS_GUIDANCE` gains a Self-Management Protocol: consult the `operant` self-skill and `operant <cmd> --help` for CLI syntax, never read the Rust source to discover flags, trust command output (no re-runs), prefer the validating CLI over hand-edited TOML, and restore the baseline after management tasks.
- **Deployment-audit closeout (R39 series)** — doctor probes the configured endpoint rather than every provider's public default (custom gateways no longer report false `✗ (invalid API key)`); the clippy gate script survives deny violations and prints the offending site; background review no longer poisons the write-origin for the process lifetime (task-local scope); memory stats read the real session count from `database.db`; the doctor key scan can no longer send a base URL as a Bearer token.
- **Secret files created 0600 at open** — the memory master-key file and channel session stores are written with `OpenOptions::mode(0o600)` in one `open(2)` instead of a post-write `set_permissions` that left a umask-default window.
- **Dead Slack `_signing_secret` removed** — R14-4 withdrawn as a misread: Slack signature verification lives in the webhook adapter (verified against `webhooks_secret`), and the Socket-Mode adapter has no signed requests to verify.
- **Tagged-release pipeline repaired** — release workflow reads `docs/CHANGELOG.md` (was a nonexistent root path) and the dead `tdg-rust` clone step is gone from build; a `v*` tag push now produces notes and artifacts instead of exiting 1.

### Changed

- **iter-716 — Clean-sweep 6: integration-truth fixes — `tool_backend` stops
  mis-advertising the live web_search chain.** The introspection tool claimed
  `tavily, exa, searxng, ddg` with `tavily` current — but the live chain is
  `sourcehound → tavily → exa → ddg → searxng` with `sourcehound` the config
  default and DDG the key-free fallback. Map + description now name sourcehound
  and default to it (static claim of the config default, noted in source).
  Stale `igs`-alias comment in web_tools.rs corrected (the config default is
  `"sourcehound"`, not "the old string"); the alias itself stays — pre-re-home
  configs depend on it reaching the key-free engine. NOT deleted (audit
  candidates re-verified as live): operant-tools `WebSearchTool` + routing feed
  the runtime agent registry consumed by the ACP orchestrator; the obscura/
  lightpanda names in browser_provider are intentional retired-name fallbacks
  with a pinned warning test. Suite: tool_backend 3/3 on clean origin/main
  (the shared tree is red from a peer in-flight core change — collect_map_nodes
  — unrelated to this delta).

- **iter-714 — Clean-sweep 5: composer text-area selection (jcode parity).**
  Shift+Left/Right select characters in the composer; Shift+Up/Down extend
  across visual rows (vim Visual mode keeps its existing Shift+arrow path);
  the selection renders reverse-video through the vendored wrap renderer
  (ui_input::wrap_input_text splits each wrapped segment into before/selected/
  after spans); Ctrl+C copies the composer selection and suppresses exit-confirm
  while one is live; any other edit key collapses it. The vendored surface
  gains `TuiState::input_selection()` (default None — no other implementor
  affected). Multibyte-safe via char-boundary clamping. Suite 1745/0 incl.
  3 selection tests (chords, backwards anchor, reverse-video render with
  surrounding chars intact).

- **iter-710 — Clean-sweep 4: Ctrl+Up/Ctrl+Down fine transcript scroll
  (jcode parity).** Completes the keyboard scroll ladder: ±3 on Ctrl+arrows,
  ±10 on PageUp/PageDown, ±20 on Alt+arrows, mouse wheel otherwise. Without
  the fine rung a mouseless session (tmux without mouse-mode, plain tty —
  wheel events arrive as nothing) could only jump in 10-line quanta; part of
  the "chat window is pinned" complaint (2026-10-09 visual audit). Both
  chords pause tail-follow; reaching the bottom resumes it. Suite 1742/0
  incl. chord-step regression test.

- **iter-709 — Clean-sweep 3: `last_msg_area` republished — the dead right-click
  context menu lives again.** The iter-648 chrome cutover deleted the
  dispatch-row writer that fed `last_msg_area`; every consumer (right-click
  message menu bounds, Ctrl+Shift+M cursor-open menu, menu clamping) gated on
  a zero rect and could never fire — production-dead since the cutover,
  flagged by the 2026-10-09 visual audit. The render pass now publishes it
  from the vendored `message_area()` alongside the other three hit-test cells
  (same post-draw discipline as the iter-661 fix). Pinned by a default-zero +
  setter-semantics unit test. Corpus verify 0 drift; suite 1741/0.

- Best-effort cross-target builds no longer veto a release, and the release no
  longer depends on a third-party submodule being reachable.
- A 7.6 MB platform-specific `libclang.deb` is no longer committed to the
  repository; the provisioner fetches it on demand.
- The installed binary is self-contained. It previously carried no rpath, so
  `~/.local/bin/operant` exited 127 with "libonnxruntime.so.1: cannot open
  shared object file" unless the shell that ran it had sourced `dev-env.sh` —
  a build artifact with a dependency on the caller's environment. The build now
  embeds a RUNPATH to `~/.local/lib/operant`, which `install.sh` populates —
  deliberately not the build tree, since an rpath into a checkout or a tmpfs
  dies the moment that directory is removed. (Static linking is not available:
  `ort-sys` cannot link the prebuilt ONNX Runtime that way.)
- **Version 0.2.0** — workspace bumped from 0.1.4 so `operant --version` distinguishes freshly built binaries from stale installs (stale builds reject configs containing newer fields).

### Verified

- Live agentic-loop E2E across the full LCM tool surface (6/6 PASS) and cross-process durability (4/4 PASS): facts saved to the global assertion scope and DAG marker in one process are recalled verbatim by a fresh process.
- Fresh-install cold-start: bundled skills seeded (29), 84 tools registered, igs 1.0.3 + obscura 0.2.0 (stealth) provisioned, AFT v0.50.0 auto-downloaded; agentmemory path (memory_save → server, smart_search recall, session/start + observe hooks) and builtin path (store/search/recall) both PASS.

## [0.1.4] - 2026-07-19

### Added

- Autonomous coding mode through `operant autonomous` and the `operant run --autonomous` compatibility alias
- Shared `[autonomous]` runtime configuration for autonomous polling interval, TODO path, status report path, validation command, git target, commit message, command timeout, and repeated-failure pause threshold
- Repo-root `TODO.md` task ledger with `Implemented` and `Pending` sections for autonomous workspace planning
- Repo-local `autonomous-status.toml` status reports that capture autonomous state, validation results, failure summaries, and last push targets
- Disposable-repo autonomous validation coverage that exercises the full tick loop without a live model call
- Long-term memory injection into agent system prompts via `<long_term_memory>` context built from durable `MEMORY.md` facts
- Async state distillation that extracts durable session facts into repo-local `MEMORY.md` after completed agent runs
- Workspace context-file auto-loading for `AGENTS.md`, `CLAUDE.md`, `.operant.md`, `HERMES.md`, and `.cursorrules` with prompt-injection scanning and truncation
- `delegate_to_sub_agent` tool for opt-in isolated child-agent delegation from the parent ReAct loop
- Headless TUI simulator (`operant tui debug simulate`) that drives the real TUI loop against a test backend, with `--dump-screen`/`--assert-screen` (rendered-screen assertions), `--agent-script` (deterministic offline mock agent events), dot-path state assertions over the App debug snapshot, and `--size`/`--max-frames` guards; documented in `docs/tui-debugging.md`
- `--dangerously-skip-permissions` flag that shows a confirmation dialog at startup and, on accept, runs the session in permission-bypass mode

### Changed

- **iter-716 — Clean-sweep 6: integration-truth fixes — `tool_backend` stops
  mis-advertising the live web_search chain.** The introspection tool claimed
  `tavily, exa, searxng, ddg` with `tavily` current — but the live chain is
  `sourcehound → tavily → exa → ddg → searxng` with `sourcehound` the config
  default and DDG the key-free fallback. Map + description now name sourcehound
  and default to it (static claim of the config default, noted in source).
  Stale `igs`-alias comment in web_tools.rs corrected (the config default is
  `"sourcehound"`, not "the old string"); the alias itself stays — pre-re-home
  configs depend on it reaching the key-free engine. NOT deleted (audit
  candidates re-verified as live): operant-tools `WebSearchTool` + routing feed
  the runtime agent registry consumed by the ACP orchestrator; the obscura/
  lightpanda names in browser_provider are intentional retired-name fallbacks
  with a pinned warning test. Suite: tool_backend 3/3 on clean origin/main
  (the shared tree is red from a peer in-flight core change — collect_map_nodes
  — unrelated to this delta).

- **iter-714 — Clean-sweep 5: composer text-area selection (jcode parity).**
  Shift+Left/Right select characters in the composer; Shift+Up/Down extend
  across visual rows (vim Visual mode keeps its existing Shift+arrow path);
  the selection renders reverse-video through the vendored wrap renderer
  (ui_input::wrap_input_text splits each wrapped segment into before/selected/
  after spans); Ctrl+C copies the composer selection and suppresses exit-confirm
  while one is live; any other edit key collapses it. The vendored surface
  gains `TuiState::input_selection()` (default None — no other implementor
  affected). Multibyte-safe via char-boundary clamping. Suite 1745/0 incl.
  3 selection tests (chords, backwards anchor, reverse-video render with
  surrounding chars intact).

- **iter-710 — Clean-sweep 4: Ctrl+Up/Ctrl+Down fine transcript scroll
  (jcode parity).** Completes the keyboard scroll ladder: ±3 on Ctrl+arrows,
  ±10 on PageUp/PageDown, ±20 on Alt+arrows, mouse wheel otherwise. Without
  the fine rung a mouseless session (tmux without mouse-mode, plain tty —
  wheel events arrive as nothing) could only jump in 10-line quanta; part of
  the "chat window is pinned" complaint (2026-10-09 visual audit). Both
  chords pause tail-follow; reaching the bottom resumes it. Suite 1742/0
  incl. chord-step regression test.

- **iter-709 — Clean-sweep 3: `last_msg_area` republished — the dead right-click
  context menu lives again.** The iter-648 chrome cutover deleted the
  dispatch-row writer that fed `last_msg_area`; every consumer (right-click
  message menu bounds, Ctrl+Shift+M cursor-open menu, menu clamping) gated on
  a zero rect and could never fire — production-dead since the cutover,
  flagged by the 2026-10-09 visual audit. The render pass now publishes it
  from the vendored `message_area()` alongside the other three hit-test cells
  (same post-draw discipline as the iter-661 fix). Pinned by a default-zero +
  setter-semantics unit test. Corpus verify 0 drift; suite 1741/0.

- TUI slash commands `/steer`, `/queue`, `/reload-mcp`, `/reload`, `/reload-skills`, and `/mouse` now perform their real actions instead of printing placeholder status (steer injects mid-turn guidance, `/reload-mcp` reconnects MCP servers live, `/reload-skills` rescans the skills directory, `/mouse` reports live capture state)
- README, `AGENTS.md`, and `CLAUDE.md` now document the autonomous workflow, the role of `TODO.md` as the workspace task source of truth, and the disposable operator workflow for validating autonomous mode safely
- Repeated autonomous failure pauses now persist across process restarts until `TODO.md` or git state changes, using `autonomous-status.toml` as the durable state store
- CLI, TUI, and autonomous sessions now reload persisted long-term memory from the current workspace before constructing agents
- The TUI workspace now uses the desktop split at 120 columns and gracefully collapses secondary panels into popups below 65 columns or 20 rows
- TUI agent reasoning now renders with quote rails, and tool activity now renders as compact blocks for clearer tool-call scanning

### Fixed

- Autonomous command execution now runs in blocking isolation with strict exit-status checks so failed validation cannot fall through to git push
- Autonomous status tracking no longer dirties workspace fingerprints or staged commits with the runtime status file itself
- TUI layout rows now preserve the primary conversation area in cramped terminals instead of letting fixed chrome starve the workspace body
- The effort picker now registers as an open modal, so background keys no longer leak through while it is visible
- `/resume` session browser now shows real last-activity timestamps and message counts instead of placeholder "just now" / 0 values
- CJK/emoji text in the ask-user dialog, status row, and new-messages indicator now wraps and sizes by display width instead of byte length
- Live TUI cost display now uses the real model-aware per-request cost from `AgentEvent::Cost` instead of a flat-rate estimate; the `/resume` session browser now shows the real accumulated cost per session (persisted via a new `Database::update_session_cost`) instead of always `$0.00` (R3 — non-streaming request path)
- Streaming mode now reports real token usage and cost too, closing the R3 gap: OpenAI-compatible providers get `stream_options.include_usage` on streamed requests and a `usage` field on the final chunk; the native Anthropic client now parses `message_start`/`message_delta` usage events. Both merge in `process_stream` and emit `AgentEvent::Usage`/`AgentEvent::Cost` the same way the non-streaming path already did
- `/tasks` now actually works as the documented alias for `/agents` in the TUI instead of printing a "not yet wired" error
- Stabilized a pre-existing parallel-test flake in `osc8`'s URL-detection tests (was racing on an unsynchronized process-global env var)
- Stabilized a matching flake in `discord_tool`'s no-token test, which was missing the `#[serial_test::serial]` attribute its sibling tests already use
- `CredentialPool::refresh_async`/`refresh_oauth_async` no longer hold a lock across the network call that refreshes each OAuth credential, which previously blocked any writer for the whole refresh loop's duration
- `operant logs --follow` (and `operant logs`) could spin at 100% CPU forever if the log file hit a read error, instead of stopping — `Lines::filter_map(Result::ok)` skips errors and can loop on a repeating error per `std::io` docs; switched to `map_while(Result::ok)`, which stops at the first error
- The context-window warning subsystem was entirely dead: `check_token_warnings()` had full 80%/95%/100% threshold logic and its doc comment said to call it after updating `token_count`, but nothing ever did — users got no warning before hitting a full context window. Wired the call into the `AgentEvent::Usage` handler; also fixed a related latent bug the wiring would otherwise have shipped live — the threshold tracker only ever escalated, so after `/clear` or `/compact` shrank usage back down, warnings would never fire again for the rest of the session. Now resets once usage drops back below the last-shown threshold
- Pasted images (`Ctrl+V` clipboard paste) showed a thumbnail row implying they'd be sent, but silently vanished on submit — the core client's request path has no multi-part/image content support yet, and nothing drained `pending_images` or told the user. `App::drop_pending_images_with_notice()` now clears them on send and pushes a warning notification instead of a silent no-op; the underlying `PastedImage`/`clear_images` plumbing is left in place for a future dedicated session to wire up real image attachment

### Removed

- 96 `cargo clippy --fix`-applied style/idiom warnings in `operant-core` (124→28) across 39 files, unblocked by fixing the one invalid clippy suggestion (`PathBuf == &str`, not a valid comparison) that had been causing the whole-crate fix to silently roll back every prior session — behavior-preserving only
- 19 more manually-judged `operant-core` warnings (28→9): 3 dead struct fields, a duplicated attribute, a doc-indent nit, and a redundant always-`Caution` branch in `skills_guard.rs`'s verdict logic (see Fixed)
- 9 manually-judged `operant-cli` warnings (136→127): an orphaned doc comment for a deleted function, a merged if/else arm, a `loop`→`while let` simplification, a manual counter→`.enumerate()`, a manual `strip_prefix`, and 2 enum variants renamed to CamelCase
- ~110 `dead_code` warnings in `operant-cli` (127→55): unused functions, methods, fields, constants, enums, and structs removed across 24 files (adapter_types.rs, app.rs, dialogs.rs, diff_viewer.rs, and 20 others), each individually verified to have zero references anywhere in the workspace, including test-only usage that a binary-only clippy scan can't see. Deliberately-parked or actually-live features (device auth flow, event-bus variants pre-published for a future registry, the model picker's real entry points) were verified and kept, not deleted
- Duplicated wraparound list-selection arithmetic across 12 TUI overlay files (`agents_view`, `effort_picker`, `hooks_config_menu`, `mcp_view`, `memory_file_selector`, `model_picker`, `plugins_hub`, `session_branching`, `session_browser`, `skills_view`, `tasks_overlay`, `theme_screen`) — each `select_prev`/`select_next` reimplemented the same "decrement, wrap to `count - 1` at zero" / "increment, wrap to `0` at `count`" logic; replaced with shared `cycle_prev`/`cycle_next` helpers in `overlays.rs` (B2), net -116 lines. Files with genuinely different selection semantics (`ask_user_dialog`'s custom-input-row wrap, `bypass_permissions_dialog`/`settings_screen`'s non-wrapping toggle/clamp) were left untouched

- Dead TUI code with no reachable callers: the legacy `ToolPermissionDialog` cluster, the `render_message` renderer family, the `RenderContext.highlight` field, and five unused `App` fields (~1,400 lines total)
- Legacy `config.yaml`/`config.local.yaml` loading from `CliConfig::load()` — `operant.toml` is now the sole file-based config source; `.env` loading and `HERMES_*` env overrides are unaffected. Also removed the now-dead `deep_merge`/`expand_env_vars_in_value` YAML-merge helpers and the redundant `"gpt-4"` model precedence heuristic in `main.rs` (env-based `HERMES_MODEL` override already covers it)
- ~125 style/idiom clippy warnings in `operant-cli`'s bin target (derived-impl opportunities, redundant closures, unnecessary borrows, `to_string` in `format!` args, simplifiable `map_or`, manual `div_ceil`, and similar), via `cargo clippy --fix` — behavior-preserving only

Full Changelog: [v0.1.3...v0.1.4](https://github.com/ishan-parihar/operant/compare/v0.1.3...v0.1.4)

## [0.1.3] - 2026-04-20

### Added

- Prompt history in the TUI input box, with `Up` / `Down` navigation that replays recent prompts and restores the current draft when you leave history browsing
- New README screenshots for the landing screen and workspace chat flow in `assets/main.png` and `assets/chat.png`
- Project-context sections in `AGENTS.md` and `CLAUDE.md` so future coding agents can immediately understand the current config, TUI, and release workflow expectations

### Changed

- **iter-716 — Clean-sweep 6: integration-truth fixes — `tool_backend` stops
  mis-advertising the live web_search chain.** The introspection tool claimed
  `tavily, exa, searxng, ddg` with `tavily` current — but the live chain is
  `sourcehound → tavily → exa → ddg → searxng` with `sourcehound` the config
  default and DDG the key-free fallback. Map + description now name sourcehound
  and default to it (static claim of the config default, noted in source).
  Stale `igs`-alias comment in web_tools.rs corrected (the config default is
  `"sourcehound"`, not "the old string"); the alias itself stays — pre-re-home
  configs depend on it reaching the key-free engine. NOT deleted (audit
  candidates re-verified as live): operant-tools `WebSearchTool` + routing feed
  the runtime agent registry consumed by the ACP orchestrator; the obscura/
  lightpanda names in browser_provider are intentional retired-name fallbacks
  with a pinned warning test. Suite: tool_backend 3/3 on clean origin/main
  (the shared tree is red from a peer in-flight core change — collect_map_nodes
  — unrelated to this delta).

- **iter-714 — Clean-sweep 5: composer text-area selection (jcode parity).**
  Shift+Left/Right select characters in the composer; Shift+Up/Down extend
  across visual rows (vim Visual mode keeps its existing Shift+arrow path);
  the selection renders reverse-video through the vendored wrap renderer
  (ui_input::wrap_input_text splits each wrapped segment into before/selected/
  after spans); Ctrl+C copies the composer selection and suppresses exit-confirm
  while one is live; any other edit key collapses it. The vendored surface
  gains `TuiState::input_selection()` (default None — no other implementor
  affected). Multibyte-safe via char-boundary clamping. Suite 1745/0 incl.
  3 selection tests (chords, backwards anchor, reverse-video render with
  surrounding chars intact).

- **iter-710 — Clean-sweep 4: Ctrl+Up/Ctrl+Down fine transcript scroll
  (jcode parity).** Completes the keyboard scroll ladder: ±3 on Ctrl+arrows,
  ±10 on PageUp/PageDown, ±20 on Alt+arrows, mouse wheel otherwise. Without
  the fine rung a mouseless session (tmux without mouse-mode, plain tty —
  wheel events arrive as nothing) could only jump in 10-line quanta; part of
  the "chat window is pinned" complaint (2026-10-09 visual audit). Both
  chords pause tail-follow; reaching the bottom resumes it. Suite 1742/0
  incl. chord-step regression test.

- **iter-709 — Clean-sweep 3: `last_msg_area` republished — the dead right-click
  context menu lives again.** The iter-648 chrome cutover deleted the
  dispatch-row writer that fed `last_msg_area`; every consumer (right-click
  message menu bounds, Ctrl+Shift+M cursor-open menu, menu clamping) gated on
  a zero rect and could never fire — production-dead since the cutover,
  flagged by the 2026-10-09 visual audit. The render pass now publishes it
  from the vendored `message_area()` alongside the other three hit-test cells
  (same post-draw discipline as the iter-661 fix). Pinned by a default-zero +
  setter-semantics unit test. Corpus verify 0 drift; suite 1741/0.

- Conversation rendering now follows the newest assistant output by default while still allowing manual scrolling with `Up`, `Down`, `PageUp`, `PageDown`, `Home`, and `End`
- Prompt mode keeps chat scrolling available through paging keys, so long replies remain readable even while the input box is focused
- Workspace UI now labels active assistant output as `responding` when `stream = false` instead of incorrectly showing `streaming`
- README now documents the prompt history keys, conversation scrolling behavior, screenshots, and release-driven documentation sync expectations

### Fixed

- Streaming provider tool-call deltas now tolerate missing `index` fields, which prevented some NVIDIA NIM tool runs from completing in `stream = true` mode
- Streaming tool-call parsing now merges incremental argument chunks and strips split `<tool_call>` XML tags from visible conversation output more reliably
- Non-streaming mode now parses XML tool calls embedded in assistant content instead of leaving tool execution text stranded in the reasoning pane
- Final assistant replies and tool outputs now land in the conversation pane more consistently instead of leaving the workspace stuck on older chat content
- Non-Windows CI and tarpaulin coverage now pass the join-error TUI test by using terminal-free run-result assertions instead of any real TTY-backed terminal setup

Full Changelog: [v0.1.2...v0.1.3](https://github.com/eikarna/operant-rs/compare/v0.1.2...v0.1.3)

## [0.1.2] - 2026-04-20

### Added

- Workspace follow-up prompting now returns to prompt mode automatically after both completed and failed runs, so a user can continue the same session without clearing history
- Regression coverage for Windows key handling, landing prompt bootstrap, follow-up prompting after errors, and activity-pane failure rendering

### Changed

- **iter-716 — Clean-sweep 6: integration-truth fixes — `tool_backend` stops
  mis-advertising the live web_search chain.** The introspection tool claimed
  `tavily, exa, searxng, ddg` with `tavily` current — but the live chain is
  `sourcehound → tavily → exa → ddg → searxng` with `sourcehound` the config
  default and DDG the key-free fallback. Map + description now name sourcehound
  and default to it (static claim of the config default, noted in source).
  Stale `igs`-alias comment in web_tools.rs corrected (the config default is
  `"sourcehound"`, not "the old string"); the alias itself stays — pre-re-home
  configs depend on it reaching the key-free engine. NOT deleted (audit
  candidates re-verified as live): operant-tools `WebSearchTool` + routing feed
  the runtime agent registry consumed by the ACP orchestrator; the obscura/
  lightpanda names in browser_provider are intentional retired-name fallbacks
  with a pinned warning test. Suite: tool_backend 3/3 on clean origin/main
  (the shared tree is red from a peer in-flight core change — collect_map_nodes
  — unrelated to this delta).

- **iter-714 — Clean-sweep 5: composer text-area selection (jcode parity).**
  Shift+Left/Right select characters in the composer; Shift+Up/Down extend
  across visual rows (vim Visual mode keeps its existing Shift+arrow path);
  the selection renders reverse-video through the vendored wrap renderer
  (ui_input::wrap_input_text splits each wrapped segment into before/selected/
  after spans); Ctrl+C copies the composer selection and suppresses exit-confirm
  while one is live; any other edit key collapses it. The vendored surface
  gains `TuiState::input_selection()` (default None — no other implementor
  affected). Multibyte-safe via char-boundary clamping. Suite 1745/0 incl.
  3 selection tests (chords, backwards anchor, reverse-video render with
  surrounding chars intact).

- **iter-710 — Clean-sweep 4: Ctrl+Up/Ctrl+Down fine transcript scroll
  (jcode parity).** Completes the keyboard scroll ladder: ±3 on Ctrl+arrows,
  ±10 on PageUp/PageDown, ±20 on Alt+arrows, mouse wheel otherwise. Without
  the fine rung a mouseless session (tmux without mouse-mode, plain tty —
  wheel events arrive as nothing) could only jump in 10-line quanta; part of
  the "chat window is pinned" complaint (2026-10-09 visual audit). Both
  chords pause tail-follow; reaching the bottom resumes it. Suite 1742/0
  incl. chord-step regression test.

- **iter-709 — Clean-sweep 3: `last_msg_area` republished — the dead right-click
  context menu lives again.** The iter-648 chrome cutover deleted the
  dispatch-row writer that fed `last_msg_area`; every consumer (right-click
  message menu bounds, Ctrl+Shift+M cursor-open menu, menu clamping) gated on
  a zero rect and could never fire — production-dead since the cutover,
  flagged by the 2026-10-09 visual audit. The render pass now publishes it
  from the vendored `message_area()` alongside the other three hit-test cells
  (same post-draw discipline as the iter-661 fix). Pinned by a default-zero +
  setter-semantics unit test. Corpus verify 0 drift; suite 1741/0.

- Runtime and operational errors in the rich TUI are now summarized in the footer while their detailed text is rendered in the `Activity` pane
- Activity entries now render as compact single-line log rows so failures stay visible in narrow panel heights
- README now documents the TOML configuration model, `operant.example.toml`, and the current ratatui-based interactive workflow

### Fixed

- Windows and PowerShell landing screen now paints an explicit dark canvas instead of inheriting a gray terminal background
- Landing prompt entry now accepts immediate typing on the first screen while still preventing duplicated characters from key-release events
- Landing status/footer no longer duplicates `idle` or `run failed`
- Current chat sessions can accept a new prompt after runtime errors without conflicting with agent self-healing logic

Full Changelog: [v0.1.1...v0.1.2](https://github.com/eikarna/operant-rs/compare/v0.1.1...v0.1.2)

## [0.1.1] - 2026-04-19

### Added

- Shared TOML-backed `AppConfig` runtime configuration across `operant-cli` and `operant-core`
- Config discovery with precedence `defaults < operant.toml/.operant.toml/config.toml < env vars < CLI flags`
- Config sections for client, agent behavior, TUI, MCP, skills, gateway, and tool/runtime defaults
- Responsive ratatui application architecture with landing and workspace views split across dedicated state, action, form, render, and app modules
- In-TUI panels for Session, MCP, Skills, and Behavior management, including modal forms for MCP server creation, skill creation, and behavior editing
- Example-config parse coverage to keep `operant.example.toml` synchronized with the Rust config schema
- GitHub release workflow that extracts matching release notes from `CHANGELOG.md` and publishes tagged build artifacts to GitHub Releases

### Changed

- **iter-716 — Clean-sweep 6: integration-truth fixes — `tool_backend` stops
  mis-advertising the live web_search chain.** The introspection tool claimed
  `tavily, exa, searxng, ddg` with `tavily` current — but the live chain is
  `sourcehound → tavily → exa → ddg → searxng` with `sourcehound` the config
  default and DDG the key-free fallback. Map + description now name sourcehound
  and default to it (static claim of the config default, noted in source).
  Stale `igs`-alias comment in web_tools.rs corrected (the config default is
  `"sourcehound"`, not "the old string"); the alias itself stays — pre-re-home
  configs depend on it reaching the key-free engine. NOT deleted (audit
  candidates re-verified as live): operant-tools `WebSearchTool` + routing feed
  the runtime agent registry consumed by the ACP orchestrator; the obscura/
  lightpanda names in browser_provider are intentional retired-name fallbacks
  with a pinned warning test. Suite: tool_backend 3/3 on clean origin/main
  (the shared tree is red from a peer in-flight core change — collect_map_nodes
  — unrelated to this delta).

- **iter-714 — Clean-sweep 5: composer text-area selection (jcode parity).**
  Shift+Left/Right select characters in the composer; Shift+Up/Down extend
  across visual rows (vim Visual mode keeps its existing Shift+arrow path);
  the selection renders reverse-video through the vendored wrap renderer
  (ui_input::wrap_input_text splits each wrapped segment into before/selected/
  after spans); Ctrl+C copies the composer selection and suppresses exit-confirm
  while one is live; any other edit key collapses it. The vendored surface
  gains `TuiState::input_selection()` (default None — no other implementor
  affected). Multibyte-safe via char-boundary clamping. Suite 1745/0 incl.
  3 selection tests (chords, backwards anchor, reverse-video render with
  surrounding chars intact).

- **iter-710 — Clean-sweep 4: Ctrl+Up/Ctrl+Down fine transcript scroll
  (jcode parity).** Completes the keyboard scroll ladder: ±3 on Ctrl+arrows,
  ±10 on PageUp/PageDown, ±20 on Alt+arrows, mouse wheel otherwise. Without
  the fine rung a mouseless session (tmux without mouse-mode, plain tty —
  wheel events arrive as nothing) could only jump in 10-line quanta; part of
  the "chat window is pinned" complaint (2026-10-09 visual audit). Both
  chords pause tail-follow; reaching the bottom resumes it. Suite 1742/0
  incl. chord-step regression test.

- **iter-709 — Clean-sweep 3: `last_msg_area` republished — the dead right-click
  context menu lives again.** The iter-648 chrome cutover deleted the
  dispatch-row writer that fed `last_msg_area`; every consumer (right-click
  message menu bounds, Ctrl+Shift+M cursor-open menu, menu clamping) gated on
  a zero rect and could never fire — production-dead since the cutover,
  flagged by the 2026-10-09 visual audit. The render pass now publishes it
  from the vendored `message_area()` alongside the other three hit-test cells
  (same post-draw discipline as the iter-661 fix). Pinned by a default-zero +
  setter-semantics unit test. Corpus verify 0 drift; suite 1741/0.

- Replaced ad hoc CLI-only config parsing with shared core config loading and runtime installation
- Moved runtime-tunable defaults and provider/tool endpoints out of scattered literals in `client`, `agent`, `gateway`, `web_tools`, `http_tool`, `terminal_tool`, and `code_execution`
- Reworked rich `run` and `chat` flows to launch the new TUI instead of the previous single-screen live monitor
- Updated build workflow artifact naming so tag builds can be promoted directly into release assets
- Bumped crate versioning to `0.1.1`

### Fixed

- Reasoning, MCP, skills, and behavior state now render as dedicated TUI surfaces instead of raw merged text
- Missing or invalid TOML configuration files now fail with user-facing diagnostics instead of silently falling back

## [0.1.0] - 2026-04-17

### Added

- ReAct orchestration loop with streaming-first architecture
- Tolerant XML parser for tool call detection with early execution
- OpenAI API client with SSE streaming support
- Dynamic JSON Schema generation from Rust structs (`schemars`)
- Tool registry with 17 built-in tools:
  - `file_read`, `file_write`, `terminal`, `code_execution`
  - `web_search` (DuckDuckGo Lite scraper), `web_fetch`, `http_request`
  - `datetime` with timezone offsets and advanced formatting
  - `memory_store`, `memory_search`, `memory_profile`
  - `todo` (in-memory task list), `clarify` (agent-to-user questioning)
  - `patch` (find-and-replace file patching)
- MCP protocol client with HTTP and stdio transports
- Persistent file-backed memory (`MEMORY.md` / `USER.md`) matching Python agent format
- Skills system with YAML front matter parsing
- Gateway adapters for Telegram, Discord, and Slack
- Context window management with compression
- RL trajectory export
- Cross-platform utilities (`platform.rs`) for shell detection, config dirs, file permissions
- CLI with `run`, `chat`, `tools`, and `test` subcommands
- 99 unit and integration tests
- CI/CD pipelines: lint (rustfmt + clippy + docs), build (3 native + 6 cross-compiled targets), test (3 platforms + coverage)

### Changed

- **iter-716 — Clean-sweep 6: integration-truth fixes — `tool_backend` stops
  mis-advertising the live web_search chain.** The introspection tool claimed
  `tavily, exa, searxng, ddg` with `tavily` current — but the live chain is
  `sourcehound → tavily → exa → ddg → searxng` with `sourcehound` the config
  default and DDG the key-free fallback. Map + description now name sourcehound
  and default to it (static claim of the config default, noted in source).
  Stale `igs`-alias comment in web_tools.rs corrected (the config default is
  `"sourcehound"`, not "the old string"); the alias itself stays — pre-re-home
  configs depend on it reaching the key-free engine. NOT deleted (audit
  candidates re-verified as live): operant-tools `WebSearchTool` + routing feed
  the runtime agent registry consumed by the ACP orchestrator; the obscura/
  lightpanda names in browser_provider are intentional retired-name fallbacks
  with a pinned warning test. Suite: tool_backend 3/3 on clean origin/main
  (the shared tree is red from a peer in-flight core change — collect_map_nodes
  — unrelated to this delta).

- **iter-714 — Clean-sweep 5: composer text-area selection (jcode parity).**
  Shift+Left/Right select characters in the composer; Shift+Up/Down extend
  across visual rows (vim Visual mode keeps its existing Shift+arrow path);
  the selection renders reverse-video through the vendored wrap renderer
  (ui_input::wrap_input_text splits each wrapped segment into before/selected/
  after spans); Ctrl+C copies the composer selection and suppresses exit-confirm
  while one is live; any other edit key collapses it. The vendored surface
  gains `TuiState::input_selection()` (default None — no other implementor
  affected). Multibyte-safe via char-boundary clamping. Suite 1745/0 incl.
  3 selection tests (chords, backwards anchor, reverse-video render with
  surrounding chars intact).

- **iter-710 — Clean-sweep 4: Ctrl+Up/Ctrl+Down fine transcript scroll
  (jcode parity).** Completes the keyboard scroll ladder: ±3 on Ctrl+arrows,
  ±10 on PageUp/PageDown, ±20 on Alt+arrows, mouse wheel otherwise. Without
  the fine rung a mouseless session (tmux without mouse-mode, plain tty —
  wheel events arrive as nothing) could only jump in 10-line quanta; part of
  the "chat window is pinned" complaint (2026-10-09 visual audit). Both
  chords pause tail-follow; reaching the bottom resumes it. Suite 1742/0
  incl. chord-step regression test.

- **iter-709 — Clean-sweep 3: `last_msg_area` republished — the dead right-click
  context menu lives again.** The iter-648 chrome cutover deleted the
  dispatch-row writer that fed `last_msg_area`; every consumer (right-click
  message menu bounds, Ctrl+Shift+M cursor-open menu, menu clamping) gated on
  a zero rect and could never fire — production-dead since the cutover,
  flagged by the 2026-10-09 visual audit. The render pass now publishes it
  from the vendored `message_area()` alongside the other three hit-test cells
  (same post-draw discipline as the iter-661 fix). Pinned by a default-zero +
  setter-semantics unit test. Corpus verify 0 drift; suite 1741/0.

- Switched TLS backend from `native-tls` (OpenSSL) to `rustls-tls` (pure Rust) for cross-compilation support

### Dependencies

- Rust MSRV: 1.86
- `tokio` 1.36, `reqwest` 0.12 (rustls), `serde` 1.0, `clap` 4.5
- `schemars` 0.8, `tracing` 0.1, `anyhow` 1.0, `thiserror` 1.0

[0.1.0]: https://github.com/eikarna/operant-rs/releases/tag/v0.1.0
[0.1.1]: https://github.com/eikarna/operant-rs/releases/tag/v0.1.1
[0.1.2]: https://github.com/eikarna/operant-rs/releases/tag/v0.1.2
[0.1.3]: https://github.com/eikarna/operant-rs/releases/tag/v0.1.3

- **iter-885 — read API v4 accepted BEFORE the kernel publishes it**
  (capability-layer audit §2, the sequencing gate). A kernel at v4 against a
  runtime that knows only v1–v3 would turn org mode OFF on the live box —
  the refusal is the runtime's protection against reading stale view shapes.
  The cross-repo order is therefore operant-first: `KNOWN_API_VERSIONS` gains
  `"4"` now, reading v1–v3 shapes unchanged; the kernel's C1/R-04 migration
  (surface units, `api_pool`, the service columns) lands AFTER this, and its
  `pack_api` lint is taught v4's columns in the same migration. Guard:
  `api::tests::a_v4_kernel_is_read_and_an_unknown_still_refuses` — a scratch
  instance publishing `4` opens; one publishing `5` still refuses.

  Label note (protocol): iters 879–884 carried two label collisions with a
  concurrent session (the peer's skill-review iters 877/878 took 877/878
  while this work was built as 877; the peer's G1 live-day docs took 884 the
  same hour). Both commits sharing a label are left on origin/main per the
  protocol; this commit notes it.

- **iter-883 — R-02: one CDP endpoint, two names — the legacy one
  still routes** (capability-layer audit 2026-10-16). The browser tools
  read `BROWSER_CDP_URL`; the legacy apply engines read `OBS_CDP_URL` —
  a department migrating its browser surface could end up with two names
  for one endpoint and no tool reading the one its config sets. One env
  seam now: `sourcehound_cdp::browser_cdp_url_from_env()` — the canonical
  `BROWSER_CDP_URL` wins, the legacy `OBS_CDP_URL` routes as a
  deprecated fallback (one-time warn), and all four readers go through it
  (browser_cdp_tool, browser_dialog_tool, browser_tool's accessibility
  tree, resolve_cdp_ws_url). Holon's browser surface manifest names
  `BROWSER_CDP_URL`. Guard:
  `cdp_env_name_r02_canonical_wins_legacy_falls_back`; the three
  env-mutating browser tests now carry `#[serial]` (they never did — the
  env is process-global and they raced anything that reads it).

- **iter-882 — R-09: a Holon body's skill list is the run's department-
  resolved library** (capability-layer audit 2026-10-16). The frozen
  prefix's `<available_skills>` block listed the org-wide standard export
  the adapter installs once into the shared `HERMES_HOME`, so a
  department's tailored skill reached the body twice — the standard copy
  in the list, the fork in the brief. `build_agent_core_with_memory` now
  roots the run's SkillManager at the directory the kernel names in
  brief.json (`skills` — `runs/<id>/skills`, department-resolved, written
  at claim; kernel half landed in Holon a5fda76): one copy of every
  skill, the fork included. A Holon brief that names none (an older
  kernel) lists no skills at all — the brief is the render, never the
  standard export. Personal mode keeps `config.skills.root_dir`
  byte-identical. Guards:
  `run_env::tests::skills_dir_follows_the_brief_and_absent_means_none_r09`
  (the brief.json seam: named → Some, absent/empty → None) and the R-07
  e2e extended — the captured provider request's `<available_skills>`
  names skills, and every listed name exists in the run's export (the
  e2e's isolated home holds no export, so the list can only come from
  the run's library).

- **iter-881 — R-08: a Holon run opens no memory provider**
  (capability-layer audit 2026-10-16). The trace resolved the audit's open
  question: the six live Holon runs left no bank anywhere because the
  adapter pinned `[memory] provider = "builtin"`, which `load_memory_manager`
  treats as legacy no-op → no provider ever existed. That pin's removal
  (kernel 293a13b) left `enabled = true` + the `agentmemory` default — which
  WOULD have opened one memory-wire bank inside the shared adapter
  `HERMES_HOME`, every employee sharing it. The gate now lives where the
  decision is made — `load_memory_manager` (main.rs) returns before the
  factory under `HOLON_RUN_DIR`, so not even the sqlite appears; the
  kernel's context bundle is the run's memory, and learnings leave through
  result.json (W6). Personal mode is unchanged (a design preference).
  Guards: `a_holon_run_opens_no_memory_provider_r08` (unit, `#[serial]`:
  no provider, no bank file under the run env; provider built in personal
  mode under the same default config) and the R-07 e2e extended — a Holon
  body's captured requests carry no `<memory_context>` and the isolated
  home holds no `memory_wire.sqlite`, while the personal run builds its
  bank under the same config.

- **iter-880 — R-10: a Holon body gets no `[continual harness]` lessons**
  (capability-layer audit 2026-10-16). `injection_block` read only the
  `[kernel] enabled` flag: nothing asked `is_holon_run()`, and the store's
  `state_dir` defaults to `~/.local/share/operant/kernel/harness` — outside
  the run's `HERMES_HOME` — against BOUNDARY.md §4 (operant keeps no
  continual-harness lesson store for org work). The Holon adapter pins
  `[kernel] enabled = false` now, but the gate lives in the runtime too:
  `injection_from` (the shared body behind `injection_block`) returns before
  the enabled flag and the per-turn cache, so a posture config that re-enables
  the kernel cannot put the body's own lessons beside the kernel's brief.
  Personal mode is unchanged. Guard:
  `a_holon_run_gates_the_continual_harness_before_its_lessons_r10` — a real
  sidecar store seeded via `refine_apply` (personal mode injects the lesson),
  then the same runtime under `HOLON_RUN_DIR` injects nothing.

- **iter-879 — R-07: a Holon body gets no workspace or global context**
  (capability-layer audit 2026-10-16). `load_context_file_prompt` appended a
  `<workspace_context>` block to every turn from two sources nobody gated on
  `is_holon_run()`: the owner's global context dir (`~/.config/operant/context`
  — outside the run's `HERMES_HOME`, so the owner's personal context) and the
  first workspace file (AGENTS.md, CLAUDE.md, …) found walking up from the
  cwd — so a body whose workdir sits in a legacy tree got that tree's stale
  protocol beside the kernel's brief: two instruction sets, one ungoverned.
  Now a Holon run loads neither (the kernel's brief is the only org guidance
  — the single-injector law); the kernel names no workspace file in
  brief.json today, so none load. Personal mode is unchanged. Guards:
  `a_holon_run_gets_no_workspace_or_global_context_r07` (unit, `#[serial]`:
  planted AGENTS.md + planted XDG global context — empty in a Holon run,
  both loaded in personal mode) and
  `a_holon_body_gets_only_the_kernels_guidance_r07` (e2e: a REAL kernel
  prepare env drives the built binary; the captured provider requests carry
  neither marker in a Holon run and both in the personal one).

