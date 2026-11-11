# Changelog

## Release index

- **iter-2105 — the tool timeout desynchronised the MCP stdio stream for the rest of the session (RT-515).** The registry wraps every tool in `tokio::time::timeout` and DROPS the future on expiry, while the MCP client read with tokio's `read_line` — documented as NOT cancel-safe — under a LONGER budget (60 s) than the registry's default (30 s). So the registry always cancelled first, the partial JSON line was lost, and every later call failed to parse; a parse failure is a PROTOCOL error (deliberately not re-issued) and the watchdog only restarts a child whose PROCESS exited, so one slow server disabled every MCP tool until the session ended. **Fixed**: the read is cancel-safe (`read_until` into a buffer that lives in the client), the namespaced MCP tools get a per-tool registry timeout (90 s) ABOVE the client's own 60 s budget so the client always times out first — cleanly, with the stream intact — and a parse failure now flags the stream as desynced so the NEXT request reconnects (without re-issuing the call). EOF is also reported instead of being spun on 256 times. The same iteration found the banner-bound test VACUOUS: its python script was a syntax error, so the server never ran. **Pinned** by `a_cancelled_read_keeps_its_partial_line_and_the_stream_stays_usable` and `an_mcp_tool_is_not_cancelled_by_the_registrys_default_timeout` — both mutation-proved — plus `a_dead_server_reports_eof_instead_of_spinning_on_the_banner_bound`.
- **iter-2102 — a dropped turn lease wedged that chat forever (RT-512).** `TurnLeaseGuard::drop` released the lease with `try_lock`; when the key map was already held the removal was skipped, the key stayed in the map, and `try_acquire` then refused that chat's EVERY later message with "⏳ Still working on your previous message" — permanently, with no log line. The comment claimed "a stale lease self-heals on the next turn" and nothing implemented a sweep. **Fixed**: the key set is a `std::sync::Mutex` (its critical sections never await, and `Drop` cannot await), and the release is unconditional and poison-tolerant — a lease can no longer be lost. **Pinned** by `a_dropped_lease_is_released_even_while_the_key_set_is_held` — the key set is held by another thread at the instant of the drop (a channel makes the contention certain) — mutation-proved (restoring the `try_lock`-and-skip release fails it by name).
- **iter-2101 — `operant service install` wrote a unit that could never start (RT-507).** The unit's `ExecStart` named a `daemon` subcommand the CLI has never had, so `systemctl --user start operant` exited 2 — and the unit's `Restart=on-failure` + `RestartSec=5` turned that into an endless restart loop. It was also a SECOND unit beside the real `operant-gateway.service`. **Fixed**: `operant service install` now delegates to the gateway installer (ONE owner of the unit text), the `start`/`stop`/`status`/`uninstall` arms manage `operant-gateway`, the macOS launchd agent runs `gateway run` instead of `daemon`, and `uninstall` also removes the legacy broken `operant.service`. **Pinned** by `the_installed_unit_names_a_subcommand_the_cli_actually_has` — the unit's ExecStart is checked against the CLI's own subcommand list (mutation-proved: with `daemon` it fails by name) — plus the launchd-argument and unit-name tests.
- **iter-1924 — an unknown image model silently served flux-pro labelled with the requested name (RT-336).** The endpoint match's fall-through arm was `_ => "fal-ai/flux-pro"` while the response still reported `"model": args.model` — a request for "banana" (a typo, or a new model name the model guessed) took a flux-pro image and returned it labelled "banana". **Fixed**: an unknown value fails loud, naming the model and the supported set (the values the field's doc already lists). **Pinned** by `an_unknown_model_fails_loud_instead_of_silently_serving_flux_pro` — mutation-proved.
- **iter-2006 — the round-15 kernel sections + the clippy gate's fixes on this round's contract needles.** A chore iteration: strict clippy (`-D warnings`) caught four needless-borrow lints on the round's own test fixtures; values unchanged and both contracts re-run green. The round's ledger/claims sections record the eight shipped rows, the sweep's five dropped candidates with the reads that killed them, and the measured lessons. (Renumbered from iter-2005: the peer independently took that id for their TUI SIGPIPE fix.)
- **iter-2073 — the `[proxy]` service key is chosen from the CONFIGURED base URL when the HTTP client is built, and the proxy is baked into that client (RT-485).** A runtime retarget (`set_base_url`) keeps the OLD provider's proxy — and its proxy credentials — for the new host. **Fixed**: the retarget's doc names the consequence. **RT-485-FOLLOW**: making the proxy follow the retarget is a design decision (rebuild the client on every retarget — a sync setter cannot take an async lock — or a per-request `reqwest::Proxy`). **Pinned** by a source contract — mutation-proved.
- **iter-2071 — an unrepresentable provider retry-after became a ZERO cooldown (RT-483).** `unwrap_or_default()` on the chrono conversion made the bench timestamp "now", so the credential pool retried the credential immediately — exactly when the provider asked it to wait. A value beyond chrono's range still means "not soon". **Fixed**: the wait is capped at a year instead of vanishing. **Pinned** by a behavioral test — mutation-proved.
- **iter-2070 — `fitted_request`'s doc described the "reported" flag as tracking the estimator's own preference, but the code hardcodes it to FALSE (RT-482).** The provider's count is exact but STALE (it is the previous request's), so the margin always applies — the body says so, the doc promised a per-request choice that never happens. **Fixed**: the doc states what the code does. **Pinned** by a source contract — mutation-proved.
- **iter-2068 — RT-475's fix shipped the HELPER but the production call site still signed the batch's first call (RT-481).** The guard was still signing one call, so the defect RT-475 described was live until now; the strict clippy gate caught it as an unused function — a dead helper is the signature of a half-applied fix. **Fixed**: the call site uses the whole-batch signature. **Pinned** by a source contract over the seam — mutation-proved.
- **iter-2067 — the error classifier's doc listed a pipeline step ("Transport error heuristics") the code does not have, and numbered the fallback accordingly (RT-480).** A reader looking for the transport heuristics would not find them. **Fixed**: the doc lists exactly the steps the code has. **Pinned** by a source contract that derives BOTH lists and compares them (the RT-435 pattern) — mutation-proved.
- **iter-2066 — `set_api_key("")`'s doc promised the CONFIGURED key back, but after a runtime retarget that key belongs to the OLD host and is deliberately never sent (RT-479).** The behaviour is right ("never send it to the new host"); the doc was false for a retargeted client — exactly the state the credential pool puts it in. **Fixed**: the doc states the retarget rule. **Pinned** by a source contract — mutation-proved.
- **iter-2065 — the SSE parser polled the inner stream before re-parsing its buffer (RT-477).** A valid event behind a corrupt payload waited for the NEXT wake-up — up to the whole idle timeout, a minute in production — and could be lost to a stall in the race where the timer was already expired. The end-of-body drain path was fixed for this (iter-754); the pending path was not. **Fixed**: the buffer is re-parsed before the stream is polled. **Pinned** by a behavioral test that measures the delay — mutation-proved (501ms against the fix's 0ms).
- **iter-2064 — two poisoned locks were handled by SILENTLY skipping the work they protect (RT-476).** The streaming ToolStart dedup set's clear was skipped on a poisoned lock (so a recovered turn's tool rows never appear — exactly what the clear prevents), and the credential pool's rotation anchor was dropped (so the rotation replaced the FIRST credential's secret instead of the one in use — exactly what the anchor prevents). The sibling locks in the same functions treat a poison as a programmer error. **Fixed**: the clear fails loudly; the anchor's plain value is recovered. **Pinned** by a source contract — mutation-proved.
- **iter-2063 — the cross-iteration repetition guard signed only the batch's FIRST tool call (RT-475).** A batch whose first call repeated while the rest changed (`[A,B]` then `[A,C]`) counted as "identical", and after six such iterations the turn was force-ended with "I repeated the same tool call 6 times in a row" — a claim true only of the first tool. **Fixed**: the whole batch is signed. **Pinned** by a behavioral test — mutation-proved.
- **iter-2062 — the Anthropic thinking branch clamped the budget with `max.saturating_sub(1024).max(1024)`, which for `max_tokens = 512` produced 1024 — ABOVE the cap (RT-474).** Anthropic requires `budget_tokens < max_tokens`, so such a request is rejected with a 400; the branch says "clamp the budget below max_tokens" and did not. It was unreachable while the shape could not be forced (RT-473) and is live now. **Fixed**: the clamp keeps its old value where it was valid and falls back to one below the cap where it was not. **Pinned** by a behavioral test over four caps — mutation-proved.
- **iter-2061 — the operator's `agent.reasoning_effort_param` NEVER reached the wire (RT-473).** The agent resolved the shape and dropped it, and the only production adapter hardcoded `Auto`: so "none" still sent `reasoning_effort` on every non-openrouter host (the very field the operator asked not to send), "anthropic_thinking" never emitted a `thinking` block, and the forced branches of `apply_reasoning_field` had no production caller at all. **Fixed**: the resolved shape travels with the request and every adapter honours it. **Pinned** by an end-to-end test (the body the adapter actually sent) + a source contract — mutation-proved on both halves.
- **iter-2059 — `[client.rate_limit] max_retries` counted ATTEMPTS, not RETRIES (RT-471).** The setting is documented (in the config and in `operant.example.toml`) as the retries AFTER the first attempt, but both retry loops used it as the total attempt count — so the shipped default of 3 made three requests instead of four: one retry fewer than the operator asked for on every transient failure. A budget of 0 still means exactly one request. **Fixed**: the attempt count is the retries plus the first attempt. **Pinned** by a behavioral test that counts the requests — mutation-proved.
- **iter-2057 — the session-row writes (the last-activity timestamp and the cost) DROPPED their Result on both paths (RT-470).** A failed write left the session list showing a stale time and a stale cost, with the agent proceeding as if the row had been updated — while the sibling counter persists (RT-365) report their failures. **Fixed**: both writes report, naming what goes stale. **Pinned** by a source contract over the class — mutation-proved.
- **iter-2056 — `is_local_endpoint` checked only the IPv4 private and link-local ranges (RT-469).** A keyless server on an IPv6 unique-local (`[fd00::1]`) or link-local (`[fe80::1]`) address was classified as a PUBLIC host, though the function's doc says "this machine or its private network": the pooled client refused the keyless passthrough and the CLI reported "missing setup step" instead of "an unauthenticated local server". **Fixed**: the IPv6 arm checks the unique-local and link-local ranges too. **Pinned** by the endpoint table test — mutation-proved.
- **iter-2055 — the auth branch kept the RAW provider body, so the sanitizer was dead on exactly one path (RT-468).** The auth error carried the provider's newlines and up to the full 64 KiB read cap, and its Display and the credential-pool's persisted bench record quoted it verbatim — while the function's own doc says the body is sanitized at construction time (newlines stripped, truncated to 500 chars), as every other branch honours. **Fixed**: the auth branch uses the sanitized body. **Pinned** by a behavioral test — mutation-proved.
- **iter-2054 — after merging a native tool call, the stream looked up the FIRST call with a non-empty name instead of the one it just merged (RT-466).** One SSE event can carry several native tool calls, so the second never got a stream-time ToolStart: its row appeared late, or never when the turn was interrupted first (the XML flush path already used the id). **Fixed**: the lookup is by the merged call's id. **Pinned** by a behavioral test — mutation-proved.
- **iter-2053 — the tool-result trim's budget is documented in CHARACTERS but the arithmetic was in BYTES (RT-465).** A CJK tool result kept ~667 characters under a 2000-character budget, and the marker reported a byte count as a character count — a wrong number the model reasons over. The repo's own token estimator is char-based and CJK-aware, and the same class was fixed in `turn_rules.rs` (RT-455). **Fixed**: the trim's arithmetic counts characters and the marker reports the characters removed. **Pinned** by a behavioral test — mutation-proved.
- **iter-2052 — the message-sequence repair MANUFACTURED `tool_calls: Some([])`, which goes on the wire as `"tool_calls": []` plus a fabricated `reasoning_content` (RT-464).** It re-created the very shape `drop_orphaned_tool_messages` removes ("providers read `Some([])` differently from `None`"): `unwrap_or_default()` yields an empty list, so two consecutive assistant messages that called nothing ended with an empty call list — a shape a validating provider rejects. **Fixed**: the repair no longer assigns an empty list, and the client's wire shape treats an empty list as no calls. **Pinned** by 2 behavioral tests — mutation-proved.
- **iter-2051 — the SPEND GUARD ran AFTER the 4096-byte absolute cap, so it never saw more than 4096 bytes (RT-463).** At the shipped default 128K window the guard's limit is ~6195 tokens (~24.8K chars) and the cap leaves ~1024 — so the guard could never fire: `agent.max_tool_result_share` was inert for every window above ~28K tokens, and the model got the generic truncation marker instead of the documented one (the result's real size, its share, its price). The code's own comment said the guard ran first. **Fixed**: the guard runs before the cap. **Pinned** by a behavioral test at the default window — mutation-proved (the fixture's published growth moved 1018 → 1025 and the cap endpoint was re-probed).
- **iter-2050 — the interrupt drain indexed a parallel id vector with the WRONG index (RT-462).** The ids came from `pending` (the calls that survived pre-flight) but were indexed with the position in `call_names` (all the calls): a pre-flight rejection shifted every pairing — the cancelled results carried the WRONG `tool_call_id`, which strict providers reject — and a batch with more calls than pending indexed PAST the end of the vector, an out-of-bounds PANIC that killed the turn task. **Fixed**: the id vector covers every call and each id sits at its own call's index. **Pinned** by a behavioral test — mutation-proved.
- **iter-2049 — the stream-retry ladder re-armed its wall clock on the turn's FIRST failure, so the first attempt's stall was EXEMPT (RT-461).** The module's own contract says the clock starts at construction, because a budget that began counting at the first drop "would exempt the very attempt that cost the most" (the 2026-09-29 incident: 60s of silence, then a second attempt) — but the production predicate re-armed on "the turn's first" failure, so that provider got a second attempt. **Fixed**: the re-arm is a named predicate that asks whether THIS attempt produced output (the progress stamp, snapshotted per attempt): a pure stall counts against the bound, a cut after real output still earns its retries (the 2026-10-13 incident). **Pinned** by a behavioral test — mutation-proved.
- **iter-2048 — the HTTP client's body reads lost the transport cause (RT-460).** A failed read of a 200 body became an empty body and a PARSE error ("EOF while parsing a value"), so a network failure was presented as garbage from the provider and classified as unknown/retryable; and a failed error-body read became an empty body. **Fixed**: the success body's read failure is a `Network` error, and the bounded error reader reports why it could not read. **Pinned** by a behavioral test (a server that declares a body it does not send) — mutation-proved.
- **iter-2047 — three more MCP protocol sends discarded their results (RT-459).** The stdio transport's error RESPONSES to a server's request (sampling, elicitation) and the SSE transport's response delivery: a server waiting for an answer never got one, and the client's log said nothing (the third-party server reported a timeout while the client looked healthy). **Fixed**: each failure is reported. **Pinned** by a source contract over the class (no protocol write or send may discard its result) — mutation-proved.
- **iter-2046 — the cron self-heal's operator-facing warning had its sentence breaks COLLAPSED into runs of spaces (RT-458).** The operator read "the stored                      schedule stays on its legacy form" — a multi-line message literal loses its breaks silently when something reflows it. **Fixed**: the breaks are restored (a line continuation). **Pinned** by a source contract (no run of 4+ spaces inside the message, a regex needle because the width is not fixed) — mutation-proved. The whole workspace was swept for the class: one instance.
- **iter-2044 — the round-18 kernel sections.** Thirteen pushed iterations (iter-2032..iter-2043) closed: two MCP notification defects, the byte-offset slug panic, the fail-open Holon write fence, the symlink-following walkers, the todo merge's dropped fields, the patch tool's blanking tier, the bundle cache's blind spot, the insights report's silent row drops, the malware gate's verb-as-package, the command fence's segment gap and false promise, the byte-measured ceiling and the deferred module's stale floor. What the round taught is in `_kernel/ledger.md` and `_kernel/claims.md` — among them: a doc and its code can disagree INSIDE ONE FUNCTION (the acceptance test decides which is right), two organs that claim to share a rule must SHARE THE CODE, and a byte/char mismatch is a CLASS, not an incident (three instances in three modules).
- **iter-2045 — the MCP OAuth token store's removal DISCARDED the deletions' errors and returned `Ok(())` unconditionally (RT-457).** A credential file that could not be removed stayed ON DISK while the function reported success, and the manager logged "removed from disk" either way. **Fixed**: the removal reports the files it could not delete (an absent file is already removed, not a failure), and the log claims the removal only when it happened. **Pinned** by 2 behavioral tests — mutation-proved. The removal path itself has no product caller (named as a residual: which command owns the credential revocation is a product decision).
- **iter-2043 — the deferred-tool module's doc put the content tokens' length floor at 3 while the code's floor is 4 (RT-456).** The file contradicted itself: the constant's own comment says the bound is 4 and why it matters (three-letter English is dominated by function words, so a lower floor made every server tool match every turn), but the module header still said three — so a caller reasoning from the header believed three-letter tokens could trigger materialisation. **Fixed**: the header states the floor the code keeps. **Pinned** by a behavioral test (a three-letter token alone must not materialise) + a source contract — mutation-proved.
- **iter-2042 — the degenerate-final-text gate measured its 80 ceiling in BYTES while the doc speaks of characters (RT-455).** A 30-character single-word CJK summary (90 bytes) was NOT flagged, so the same reasoning-leak artifact the gate catches in ASCII was surfaced as a final answer — and the threshold depended on the script. **Fixed**: the ceiling counts characters. **Pinned** by a behavioral test (the same artifact must be flagged in either script) — mutation-proved.
- **iter-2041 — the Holon command fence judged only the FIRST token of a line, so a SHELL line could smuggle a second command past the policy's allowlist (RT-454).** `git status; rm -rf /` named only `git`, passed the allowlist, and the shell then ran BOTH — while the kernel's allowlist (the fence's stated reference) judges EVERY segment. Its doc also promised a shell re-tokenization the code never did (the acceptance test pins the opposite). **Fixed**: the fence is told whether the line runs through a shell and judges that line's segments one by one, using the kernel's own (now shared) splitter; without a shell the first token is the binary, unchanged. **Pinned** by a behavioral test + a doc contract — mutation-proved.
- **iter-2040 — the OSV malware gate checked the runner's VERB as the package (RT-453).** `pipx run black` looked up "run", `pipx install black` looked up "install", `npx -y @scope/pkg` looked up "-y" — none exist, so the lookup found no advisories and the tool reported `known_malware: false` for packages it never checked (the repo's own test pinned the wrong value). **Fixed**: the parser skips the verbs and flags and takes the package that follows (a flag that takes a value yields its value). **Pinned** by 2 behavioral tests — mutation-proved.
- **iter-2039 — the insights report DROPPED unreadable session rows silently, and turned a bad counter into a silent 0 (RT-452).** The rows were collected with `filter_map(|r| r.ok())` and the counters with `unwrap_or(0)`, while the query itself succeeded → `error: None` — so a partially unreadable database produced wrong totals (sessions, messages, tokens, cost) with no signal that anything was missing. The RT-294 contract, failing at row granularity. **Fixed**: the dropped rows are NAMED in the report's `error`, a NULL counter is still the schema's default, and a wrong-typed counter makes the row unreadable. **Pinned** by a behavioral test — mutation-proved.
- **iter-2038 — the skill-bundle cache was keyed on the DIRECTORY's mtime, so an EDITED bundle was never re-read (RT-451; renumbered from RT-450 when the peer took that id).** On Linux, editing a file's content does not change the parent directory's mtime (only create/delete/rename does), while the doc promises "a bundle was added, edited, or removed → the next call transparently rescans" — so stale guidance kept being injected into every turn for the life of the process. **Fixed**: the cache key carries the newest FILE's mtime too. **Pinned** by a behavioral test (edit in place, read again with no explicit refresh) — mutation-proved.
- **iter-2037 — the patch tool's three tiers disagreed on an EMPTY replace (RT-449).** The contract says an empty `replace` on a whole-line `find` means "remove that line" ("so the tool could blank a line and never remove one"); the fuzzy tier and the default tier honoured it, the CHOSEN-occurrence tier substituted the empty string and BLANKED the line. **Fixed**: the chosen occurrence now deletes the line (a fragment delete still removes just the fragment). **Pinned** by 3 behavioral tests — mutation-proved. The module's temp-file helper was also hardened (a nanos name could collide between parallel tests).
- **iter-2036 — the todo tool's MERGE mode dropped `details`, `priority`, `assigned_to` and `blocked_by` (RT-448).** The merge copied only content/status/phase/lane, so an update that named a new brief or owner kept the OLD values — the model was told the item was updated while the stored row (and the TUI card) kept the stale ones. **Fixed**: the merge carries the four fields, under the rule phase/lane already follow (an omitted field leaves the old value). **Pinned** by 2 behavioral tests — mutation-proved.
- **iter-2035 — the file tools' recursive walkers FOLLOWED symlinks; the repo's own rule says they must not (RT-447; renumbered from RT-446 when the peer took that id).** `file_state`'s collector and `file_tools`' search and list walkers all asked `path.is_dir()`/`path.is_file()`, which resolve a symlink — `checkpoint_tool.rs` documents the safe form (`entry.file_type()`). A symlinked FILE was read THROUGH (the search returned a sensitive file's content, past the deny list), and a symlink CYCLE made the walkers walk the same tree at dozens of levels (82 entries instead of 2). **Fixed**: all three use the no-follow form. **Pinned** by 4 behavioral tests — mutation-proved. The sweep's "stack overflow" hypothesis was NOT proven (the path length bounds it) and the row states what was measured.
- **iter-2034 — the Holon policy-roots WRITE fence FAILED OPEN when the target's parent directory did not exist (RT-445).** The fence canonicalized the path; a missing parent made that return `None` and the fence returned `Ok(())` ("the strict checks below handle it" — they have NO roots check), and the write tool creates the missing parents — so a governed run could write outside its roots with no refusal recorded. **Fixed**: the fence resolves through the nearest existing ancestor and refuses an unresolvable path (fail closed). **Pinned** by a behavioral test (refused AND recorded; a missing-parent path inside the roots still passes) — mutation-proved.
- **iter-2033 — `peel_revision` split the model slug at a BYTE offset, so a slug whose tail cut a multi-byte character PANICKED (RT-444; renumbered from RT-443 when the peer took that id).** `slug.split_at(slug.len() - 9)` panicked ("byte index is not a char boundary") on a slug like `a日日日日x`; a provider registry entry's `model` field reaches it through `RuntimeKey::parse_in` on the runtime's fallback path. **Fixed**: both splits are char-boundary guarded (a date suffix is ASCII, so a non-boundary offset cannot be one). **Pinned** by a behavioral test plus the ASCII forms still parsing — mutation-proved.
- **iter-2032 — two MCP notification defects: two of the three `send_notification` implementations could NOT fail (RT-441), and the streamable-HTTP `notifications/initialized` was sent WITHOUT the session id (RT-442).** The stdio transport discarded the write and the flush and silently skipped a missing transport; the streamable-HTTP one discarded the whole send (a 500 was invisible); only the SSE one reported. They carry the protocol's `notifications/initialized` — a lost one leaves the client believing it is connected to a server that was never told, and without the echoed `Mcp-Session-Id` the notification landed in a NEW session. **Fixed**: both transports report (matching the SSE one) and the notification mirrors the request path's headers. **Pinned** by a behavioral test, a source contract over the class, and the streamable handshake test (whose mock now REQUIRES the session id) — mutation-proved. Renumbered from RT-439/RT-440 when the peer took those ids.
- **iter-2031 — the round-17 kernel sections + the strict-clippy gate's fix on this round's own fixture.** A chore iteration: the gate caught one unused binding in the round's own injector fixture (the binding was for its side effect) — fixed and the gate re-run green. The round's ledger/claims sections record the TWELVE shipped rows (RT-421..RT-425, RT-426..RT-438) with their mutation proofs and the round's measured lessons — a crate-wide sweep finds the CLASS a single report cannot; a rollback that discards its result leaves the REFUSED state in place; a pre-existing test can pin the DEFECT; a loud error that feeds a permissive default is still the widening the docs forbid; a unit mismatch is a silent content loss; a cursor that jumps over pending items DELETES them, and the fix must hold with a MONOTONIC cursor; a guard that compares a NAME misses every ALIAS of it; a field name can make a wrong doc BELIEVABLE; a BEHAVIORAL test alone cannot catch a DOC regression; a doc that lists an enumeration can be CHECKED AGAINST THE CODE; an operator-facing message with a negative number is a bug; a dated correction that QUOTES the claim re-creates it; a needle that spans a message's own line break matches NOTHING; a PART-reverted fix can be MASKED by another path; the peer takes ledger ids CONCURRENTLY.
- **iter-2030 — the cosine similarity's doc claimed the range is [0, 1]; the function returns the RAW cosine (RT-438).** Opposite vectors score -1; the bounded range is a CALLER's policy (`context_management.rs` clamps with `.max(0.0)`; `context/lcm.rs` sorts descending). **Fixed**: the doc states the raw range and names the clamp. **Pinned** by a behavioral test plus a doc contract — mutation-proved (the first mutant showed a behavioral test alone cannot catch a doc regression).
- **iter-2029 — the context-injection schema comments named class sets that are neither live nor retired-honestly (RT-437).** The watermarks comment listed `dm | global | dept | self` (the last three are retired; the live `feed` was missing) and the items comment called `feed` a "future" source — it is live. **Fixed**: both name `dm | feed`. **Pinned** by a behavioral test (a feed item is written) plus a source contract — mutation-proved.
- **iter-2028 — the context injector's doc sent a reader to the wrong database, and the field name sent them there (RT-436).** The doc said the state lives in the app database; `open` resolves the ORG SIBLING (`operant_kanban.db` beside it) — the exact "invisible no-op" `open`'s own doc recounts. **Fixed**: the doc names the org sibling and the field is `db_path`. **Pinned** by a source contract plus a behavioral test (the org sibling is the file that exists) — mutation-proved.
- **iter-2027 — the learning annotation's documented kind list named a kind nothing writes and missed the one that is (RT-435).** The live kinds are `skill_changes`, `learnings` and `learning_changes` (the knowledge-base edits); nothing writes `proposals` — so a consumer keying on the documented list missed every knowledge-base edit and waited for proposals. **Fixed**: the note names the live kinds. **Pinned** by a SELF-MAINTAINING contract (the kinds are derived from the emitters and compared with the note) — mutation-proved.
- **iter-2026 — the `Yolo` seat mode's doc named ONE exception; the seat's OWN deny list bites under Yolo too (RT-434).** The file's own precedence list calls the deny list "absolute over everything below" and `decide` applies it before the Yolo branch, so a seat with a deny row does not get "every tool open" — the doc said it did. **Fixed**: the doc names both exceptions. **Pinned** by a source contract (the behaviour was already pinned) — mutation-proved.
- **iter-2025 — the turn-attribution module's note named the type the delivered event does NOT carry (RT-433; renumbered from RT-432 when the peer took that id).** `TurnEnd::attribution` is an `Option<TurnAttribution>` (the three-state classification); the note said the bare `TurnBreakdown` with a stale size, while the module's own header said otherwise. **Fixed**: the note names the delivered type and the measured size (104 bytes). **Pinned** by a source contract plus a measured size assertion — mutation-proved.
- **iter-2024 — the turn-attribution docs claimed the spans were "exclusive" (RT-431).** `Complete` means every span was MEASURED (the classifier never consults `is_exclusive`), and the breakdown's residual saturates at zero, so a turn whose measured spans exceed its wall clock is `Complete` while its spans sum to MORE than the total. **Fixed**: both docs state the truth. **Pinned** by 2 behavioral tests plus a doc contract — mutation-proved.
- **iter-2023 — the kanban subscriber log claimed a notification that never happened (RT-430).** `notify_subscribers` logged an imperative line naming the task, platform, chat, event and payload as though the subscriber had been told; it sends nothing (the manager holds only the subscription database) and has no caller in the workspace. **Fixed**: the log says the subscription matched but was NOT delivered. **NAMED**: wiring the delivery is a product decision (RT-430-FOLLOW). **Pinned** by a source contract — mutation-proved.
- **iter-2022 — the stale-task diagnostic reported "-1s" for a task that had NEVER heartbeated (RT-429).** `hb.map(…).unwrap_or(-1)` produced the operator-facing message "has been in 'running' status for -1s without heartbeat" — a negative elapsed time for exactly the case the check flags. **Fixed**: the elapsed time is the one since the last heartbeat, or since the task STARTED when it never sent one (the dispatcher stamps `started_at`); when even that is unknown the message states no duration. **Pinned** by a behavioral test — mutation-proved.
- **iter-2021 — `next_round` promised `None` when the retrieval had completed, but an EARLY completion was never recorded (RT-428).** The round reported `complete: true` when the evidence requirement was met, but the state kept no completion flag, so the guard could only fire at the round cap — a retrieval that finished in round 1 got another recall. **Fixed**: the state records the completion and `next_round` honors it. **Pinned** by a behavioral test — mutation-proved.
- **iter-2020 — a DENIED seat escalation carried no resolution string (RT-427).** `SeatApprover`'s doc promises the resolver's name is recorded on the resolution string an operator sees; `approve` returned one, `deny` returned `Ok(())` — so on a denial the resolver was named nowhere and the operator saw a fixed "Action denied." **Fixed**: `deny` and `expire` return the same shape of resolution, and the gateway surfaces it. **Pinned** by a behavioral test — mutation-proved.
- **iter-2019 — four writes whose failures were discarded: the run's refusal record, its learning record, the refused skill write's rollback, and the half-created skill cleanup (RT-426).** A policy denial could vanish with no audit trail, a learning annotation could vanish, and a skill write the security scan REFUSED could stay on disk with the agent told only that the scan refused it. **Fixed**: all four report, and the refusal now names the still-on-disk consequence (the rollback became fallible so its failure can reach the caller). **Pinned** by 2 behavioral tests plus a source contract for the class — mutation-proved.
- **iter-2018 — the round-16 kernel sections + the clippy gate's fixes on this round's own test fixtures.** A chore iteration: strict clippy (`-D warnings`) caught two `field_reassign_with_default` lints on the round's own context-injection fixtures; values unchanged and both contracts re-run green. The round's ledger/claims sections record the TEN shipped rows (RT-416..RT-425) with their mutation proofs and the round's measured lessons — a library-target gate cannot see a link defect; a host-detected dependency is a build-shape dependency; "serialized by ENV_LOCK" was true of the file, not of the crate; a needle that names an instance misses a renamed one; a dated correction that quotes the claim re-creates it; a guard that compares a NAME misses every ALIAS of it; a doc can be false in the SAFE direction; "an error on read" is not "the run path does not widen"; a pre-existing test can pin the DEFECT; a unit mismatch is a silent content loss; a cursor that jumps over pending items deletes them, and the fix must hold with a MONOTONIC cursor; a partial revert can be MASKED by another path.
- **iter-2017 — the context collector took the NEWEST pending items and the cursor jumped past the rest (RT-425).** A class with more pending items than the per-class limit had its OLDER items fall below the cursor the moment the newest were collected: never collected, never rendered, no trace — while the module doc promises "at-least-once semantics". **Fixed**: the query takes the OLDEST pending items, so the cursor walks through the backlog. **Pinned** by a behavioral test (25 pending DMs, all collected) — mutation-proved.
- **iter-2016 — the context-injection quotas are documented in characters but charged in bytes (RT-424).** `total_char_cap` ("chars") and the per-aspect "char quotas" were metered with `text.len()` (bytes), so a 40-character emoji DM (160 bytes) was charged against a 60-character quota, overflowed the class quota, went to the pool, overflowed the total cap, and was DROPPED — non-ASCII content silently disappeared from the seat's prompt. **Fixed**: the accounting counts characters at all three charges. **Pinned** by a behavioral test — mutation-proved.
- **iter-2015 — a corrupt seat-policy row widened the seat to ungoverned, the widening the module doc says is impossible (RT-423).** `SeatPolicyDb::policy_for` logged the read error and returned `None` (= no policy = ungoverned), so a typo'd `lockdown` spelling re-opened every SAFE tool the seat was shut out of. **Fixed**: the seam FAILS CLOSED (lockdown — every tool escalates) and the log says so. **Pinned** by a behavioral test (the verdict measured through `decide` for a safe tool) and the updated pre-existing assertion — mutation-proved.
- **iter-2014 — `delete_board("")` deleted the default board's database, and the roots fence's note claimed an empty list was ungoverned (RT-421, RT-422).** `resolve_path` treats `""` as an alias of `"default"`, but the delete guard compared the slug — so `operant kanban delete-board ""` deleted the default board's entire database; the guard now compares the resolved path. And `holon_policy_roots` returns `Some([run_dir])` for an empty `roots` list (the fence stays ACTIVE, upstream's "the workdir only") while the note said None/ungoverned, and the sibling note claimed the two fences shared one convention. **Fixed** both; **Pinned** by 4 contracts (3 behavioral) — mutation-proved.
- **iter-2012 — the crate had THREE locks for one process-wide environment (RT-420).** Observed as a real suite failure ("Source profile 'source' does not exist"): `env_passthrough.rs` and `sourcehound_cdp.rs` each held a LOCAL `ENV_LOCK`, which serializes only their own file — and `sourcehound_cdp.rs` mutated `BROWSER_CDP_URL` under it while `tools/browser_cdp_tool.rs` mutated the same variable under the shared lock. **Fixed**: the local locks are gone; every shared-variable mutation holds the crate's one lock. **Pinned** by two contracts — mutation-proved. The suite is green (48 targets) with `test_clone_profile` passing.
- **iter-2011 — two more swallowed permission writes: the OAuth directory's 0700 and the downloaded bridge binary's 0755 (RT-419).** The class behind RT-418, fixed once: one shared helper (`fs_secrets::set_mode_reporting`) applies the mode and reports the failure with the consequence the caller names; both sites route through it. **Pinned** by a behavioral test (the helper surfaces the error) plus a class contract — mutation-proved.
- **iter-2010 — the config write's mode round-trip was documented as unconditional but its failure was discarded (RT-418).** "The mode … must survive the round-trip" was not guaranteed: a failed `set_permissions` left the temp file at 0600 and that is what the rename put in place — the user's config silently changed permissions. The fallback is the SAFER mode, so the fix reports rather than changing the policy. **Pinned** by `the_config_mode_round_trip_reports_a_failed_carry_over` — mutation-proved.
- **iter-2009 — the deployability plan's other two Phase-B claims were stale in the same way (RT-417, scope completed).** The plan called the installer an unchecked release build of `operant-cli` (it checks `cmake`/`pkg-config` and dies with one actionable sentence naming the package — the finding's own accept criterion) and called the env scripts carriers of one developer's absolute paths (`dev-env.sh` derives every path from `LOCAL_DIR` now). **Fixed**: both corrected with the same dated marker; the contract extended and the mutation re-run.
- **iter-2008 — the deployability plan still claimed the build needs a libsonic artifact (RT-417).** B1 asserted `build.rs` emits `cargo:rustc-link-lib=sonic` (deleted at iter-499) and B2 asserted only the repo-local `libsonic.a` rescues the link (after RT-416 the link needs none). **Fixed**: dated corrections at the top and in both findings; B2's accept criterion is now met by the stronger form. **Pinned** by `the_deployability_plan_does_not_claim_the_link_needs_libsonic` — mutation-proved.
- **iter-2007 — the operant-cli binary did not LINK on any host with libsonic installed (RT-416).** espeak-ng DETECTS a system libsonic (detected, not configured), so `USE_LIBSONIC` compiles seven `sonic*` references into the archive; espeak-rs-sys's build script forwards only the libraries it built, never espeak-ng's PRIVATE system dependency, so nothing emits `-lsonic` and `cargo build -p operant-cli` exited 101 with undefined `sonic*` symbols. The repo's own build.rs carried the false premise ("Nothing references a sonic symbol"). **Fixed**: the seven symbols are stubbed in the existing `espeak_audio_stubs.c` — the audio backend is stubbed anyway, so the speed-up has nothing to speed up, and no host library or header is needed. **Pinned** by `native_link_stubs` (3 contracts) — mutation-proved (the stubs removed → the contract fails by name AND the link reproduces the exact undefined symbols).
- **iter-2004 — the native-cert loader reported a count where an operator needs the entry, and discarded the store's rejections (RT-414).** rustls-native-certs' promise ("a bad store entry is skipped, not fatal") was silent on both halves: load errors reduced to a count (no way to identify the offending entry when debugging a TLS failure), and the per-cert trust-store rejection discarded outright — the TLS config is built from whatever subset parsed. **Fixed**: both report. **Pinned** by `the_cert_load_paths_report_what_they_skip` — mutation-proved.
- **iter-2003 — the token response's scope was read and thrown away by an unnamed discard (RT-413).** `parse_token_response` drops the RFC 6749 §5.1 payload's `scope` with no note, so the granted scope never reaches any caller of either flow and a narrowed-grant response is indistinguishable from a full one. **Fixed**: the parse site names the drop and its consequence. **NAMED**: adding a scope field to `OAuthTokenResponse` would grow an API no consumer reads — what to do with a narrowed grant is the credential store owner's decision. **Pinned** by `the_dropped_scope_is_named_at_the_parse_site` — mutation-proved.
- **iter-2002 — the SMTP timeout protection was conditional on two discarded fallible setters (RT-412).** The comment promises a 30s timeout so a hung SMTP server can't block the gateway forever, but `set_read_timeout`/`set_write_timeout`'s results were both discarded — so a server that stops responding mid-conversation blocks the conversation's blocking thread indefinitely with no timeout applied and nothing logged. **Fixed**: both applies report at warn, naming the consequence. **Pinned** by `the_smtp_timeout_setters_report_a_failed_apply` — mutation-proved.
- **iter-2001 — the refresh coordinator's follower comment claimed a recovery that cannot run — a panicking leader wedges every follower FOREVER (RT-411).** "Sender dropped without a result — reachable only if the leader's future panicked" is false: `Flight` owns the `watch::Sender` and both the coordinator's map and the follower hold an `Arc` across the await, so the sender cannot drop while a follower waits (probe-verified: `changed()` still times out after the map entry is removed). **Fixed**: the comment names the wedge. **NAMED**: waking followers on a leader's death is real machinery (a panic cleanup or a bounded wait) — a design decision. **Pinned** by `the_follower_comment_names_what_the_dead_arm_actually_does` — mutation-proved.
- **iter-2000 — the crate's doc links had no checker: four dangling symbols, one a guard that does not exist (RT-410).** A new gate (`RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links" cargo doc --no-deps -p operant-core`) was RED on the crate — the campaign's first run of it. Four links dangled: the RT-407 fix itself re-armed the rot by replacing a stale line NUMBER with a fresh one (now a symbol anchor), `estimate_tokens` and `TurnEnd` pointed at unqualified homes (now anchored), and `composites_match_the_live_server` named a test that does NOT exist (the doc now names the four guards that do and states that none compares the live server). **Pinned** by two contracts — both mutation-proved.
- **iter-1999 — the device-flow `poll` doc promised a refresh-token seed the signature does not carry (RT-409).** "`previous_refresh` seeds the returned refresh token for servers that omit it from the response" — no such parameter exists and the parse passes no seed, so a device-code response without a `refresh_token` yields an unrefreshable credential. The parser under it (used by the auth-code flow) DOES support the seed. **Fixed**: the doc states what the path does. **NAMED**: threading a real parameter is a signature change to a public flow with no production caller (established by every call spelling) — the owner's call. **Pinned** by `the_poll_doc_names_the_absent_seed_mechanism` — mutation-proved.
- **iter-1997 — the webhook doc advertised a route that does not exist and cannot (RT-408).** The route table read "POST /webhook/notion — …hands the event to the kernel's notion adapter" while the arm mounts at the instance's SECRET path — a literal "notion" route is exactly the guessable path the CLI's O-08 note (token-rotation guard) rules out, and the 2026-10-15 audit asserts a POST to it is a 404. An operator wiring Notion to the advertised URL sent every event into the generic `IncomingMessage` path. **Fixed**: the doc names the secret mount point and cites the CLI-side evidence. **Pinned** by `the_route_doc_names_the_secret_mount_point` — mutation-proved.
- **iter-1996 — a repair: the RT-407 ledger row cited the stale module-export line the RT-209 guard forbids, turning the suite red.** The round's own suite run caught it: my "verified reachable" list re-introduced the exact defect RT-209's guard prevents. **Fixed**: the list cites the export (a module path, the stable claim); all four guarded docs are clean again and the guard is green. Lesson: a fix row's own prose is a doc, checked by the same guards.
- **iter-1993 — the model-roles header cited a line that no longer holds the definition it names (RT-407, renumbered from RT-405/RT-406 after the peer took both).** The header cites `agent/events.rs:491` as the home of `OperantAgent::set_model`; line 491 is blank (the definition is at 540), so a reader following the doc's own pointer could conclude the self-switching behaviour does not exist. **Fixed**: the citation lands on the definition again; a crate-wide citation sweep found the four other cites reachable. **Pinned** by `the_doc_citation_lands_on_the_definition_it_names` (which follows the doc's own pointer) — mutation-proved.
- **iter-1992 — the round-14 kernel sections + the clippy gate's fixes on this round's contract needles.** A chore iteration: strict clippy (`-D warnings`) caught three lints on the round's own test fixtures; values unchanged, the same needles assert the same facts, and the round's contracts re-run green. The round's ledger/claims sections record the eleven shipped rows, the three dropped candidates with the reads that killed them, and the measured lessons.
- **iter-1991 — the `GuardScanner` constructor doc claimed the patterns compiled in the initializer (RT-404).** The body constructs nothing; the ~80 regexes compile in the `THREAT_PATTERNS` `LazyLock` on the FIRST scan, so a reader expecting uniformly cheap scans met a one-time pause with nothing naming the cost. **Fixed**: the doc names where the compile lands. **Pinned** by `the_constructor_doc_names_where_the_patterns_compile` — mutation-proved.
- **iter-1990 — the inline-shell timeout marker reported the request, not the deadline that fired (RT-403).** The marker read "timeout after {timeout_secs}s" while the enforced deadline is `timeout_secs.max(1)` (the 1s floor), so a caller passing 0 got a 1s kill labelled "timeout after 0s" — a number nobody used. **Fixed**: the marker names the enforced deadline; the doc states the floor. **Pinned** by `the_timeout_marker_reports_the_enforced_deadline` (behavioural) — mutation-proved (the mutant printed "timeout after 0s: sleep 3").
- **iter-1989 — the skill file walk followed symlinks — the scan and the content hash read OUTSIDE the skill tree (RT-402).** `collect_files`' doc promises "all regular files" but used `is_dir()`/`is_file()`, which follow: a symlinked directory was traversed (reading host content into the scan and the hash) and symlinks were pushed as regular files; the all-entries walk could recurse forever on a symlink cycle. **Fixed**: `symlink_metadata` (no follow) for the file walk, a canonical-path visited set for the entry walk. **Pinned** by two behavioural tests — mutation-proved (the mutant collected `["SKILL.md", "link.txt", "host-secret.txt"]`).
- **iter-1988 — `content_hash`'s doc claimed coverage the digest does not have (RT-401).** The doc said the digest covers "all files" while it covers only the files whose bytes could be read. **VERIFIED FALSE before shipping**: the sweep's stronger claim (a file becoming unreadable leaves the digest unchanged) is false — the digest changes when a file's bytes stop contributing, so my perturb-the-digest fix was reverted (its mutation run came back green). **Fixed**: the doc says what is measured, with the false claim recorded. **Pinned** by `the_hash_doc_names_what_the_digest_measures` — mutation-proved.
- **iter-1987 — the `too_many_files` count counted directories and symlinks while reporting "files" (RT-400).** `MAX_FILE_COUNT`'s doc limits "files" and the finding reports "skill has N files", but the counter incremented per collected entry before the symlink/is_file guards — a skill of 51 empty subdirectories tripped the limit and reported "52 files" (`total_size` by contrast sums real files). **Fixed**: the counter increments past the guards. **Pinned** by `the_file_count_counts_files_not_directories` (behavioural both directions) — mutation-proved.
- **iter-1986 — the scanner's extension gate framed a curated list as "text files only" (RT-399).** The gate is a curated 23-extension list; every other text format (`.java`, `.rs`, `.go`, `.c`, …) is skipped, so `java_runtime_exec`'s pattern is unreachable — no `.java` file is ever read. **Fixed**: the docs say what the gate does and name the gap; extending the list is a named policy decision (RT-399-FOLLOW), not smuggled into a doc fix. **Pinned** by `the_extension_gate_docs_name_what_is_and_is_not_scanned` — mutation-proved.
- **iter-1985 — the skill scanner failed open — an unscannable file or path scanned "clean" (RT-398).** A scannable-extension file that was not valid UTF-8 returned zero findings (so a payload in a latin-1 "text file" scanned Safe and was ALLOWED), and a path that was neither dir nor file ran no branch and reported "clean scan, no threats detected" for a skill never scanned. **Fixed**: scannable files read lossily from their bytes; unreadable files and unscannable paths each produce a Medium `scan_gap` finding so the verdict is never Safe-by-omission. **Pinned** by two behavioural tests — mutation-proved (the mutant showed `scan_verdict: Safe` for an exfil payload).
- **iter-1984 — the `Trusted` skill tier matched any source PREFIXING a trusted repo (RT-397).** The doc reserves the tier for "openai/skills and anthropics/skills only" but the classifier used `starts_with`, so "openai/skills-evil/x" was classified Trusted and upgraded to the first-party install policy. **Fixed**: the match is a path-component match. **Pinned** by `only_genuinely_trusted_repo_paths_get_the_trusted_tier` (behavioural) — mutation-proved.
- **iter-1983 — the inline-shell cap was in bytes over a char label — a multibyte snippet panicked the turn (RT-396).** The doc says the cap is in chars, but `String::len()`/`truncate()` count bytes, so CJK/emoji snippet output hit byte 4000 mid-character and `truncate` PANICKED inside `run_inline_shell` (the guarding test only fed ASCII). **Fixed**: the cap holds in characters. **Pinned** by `run_inline_shell_truncates_multibyte_output_on_a_char_boundary` (behavioural) — mutation-proved (the mutant panicked at the production line with the char-boundary assertion).
- **iter-1982 — the active-session lock writes were discarded, breaking the module's own eviction promise (RT-395).** The doc promises "their locks are refreshed on every message, so a busy session can never be evicted by a new one"; both writes behind it were discarded (the refresh sent an empty body; a new lock was created with no timestamp — born stale). **Fixed**: both report naming their consequences. **Pinned** by `the_lock_writes_report_a_failed_refresh` — mutation-proved.
- **iter-1980 — four `gateway_session` state changes discarded their persist (RT-394).** suspend / resume-pending mark / resume-pending clear / activity timestamp each used `let _ = self.save_entry(&entry)`, so a failed write left the disk entry on its previous state with no log: a suspended session returned after restart, a cleared resume flag stayed set (a duplicate resume), a stale timestamp forever. **Fixed**: a shared helper reports naming both sides. **Pinned** by `the_entry_writes_report_a_failed_persist` — mutation-proved.
- **iter-1979 — the round-13 kernel sections (ledger + claims).** A docs iteration: the sweep's two dropped candidates named with the reads that killed them, the ten shipped rows with their mutation proofs, and the round's measured lessons (the fourth consumed `#[cfg(test)]`, self-satisfaction instances six through ten, the unpinned seam, two vacuous fixtures, two compile-error "proofs").
- **iter-1978 — the ledger repair fold (the iter-1968 note parses as a second RT-384 section).** A chore iteration: the repair record now lives inside the RT-384 section it repairs; the index check is green again.
- **iter-1977 — the skill summary sent raw file lines as the description the doc promises (RT-393).** `SkillSummary::description`'s doc promises "Skill description from SKILL.md frontmatter"; the producer sent the first ten raw lines — delimiters and `name:`/`version:` rows included — so the LLM saw frontmatter noise. **Fixed**: a parser extracts the `description:` field. **Pinned** by `the_summary_description_is_the_frontmatter_field` (behavioural on the parser AND the producer seam) — mutation-proved.
- **iter-1976 — three curator docs promised a stale lifecycle state that does not exist (RT-392).** The header's "active → stale → archived", run_review's "marks those past `stale_after_days` as stale", and apply's walk doc all promised a transition `LifecycleState` has no variant for — staleness is a report classification only. **Fixed**: the docs describe the transitions that exist. **Pinned** by `the_stale_lifecycle_claims_match_the_code` — mutation-proved.
- **iter-1975 — `repair_schedules`' doc claimed an interval expansion the schedule module forbids (RT-391).** The self-heal's doc read "normalizes stored schedules (5-field / "every Nh" → 6-field)" while interval phrases are durations persisted RAW (the 2026-10-24 expansion bug is why). **Fixed**: the doc says what the heal normalizes and why phrases keep their form. **Pinned** by `the_repair_doc_names_the_transformation_that_actually_happens` — mutation-proved.
- **iter-1974 — the turn-summary doc promised a display bucket no formatter renders (RT-390).** `VERB_GROUPS`' doc said unlisted tools "fall into a generic 'called N tools' bucket"; no code path renders one — they fold into the "+N more" tail. **Fixed**: the doc names the real destination. **Pinned** by `the_verb_groups_doc_names_the_tail_the_tools_really_land_in` — mutation-proved.
- **iter-1973 — the session `discard` doc promised a durable delete the body never performs (RT-389).** The doc read "Delete a session's transcript from memory and disk … the only operation here that loses data" while the body only removes the resident copy (the session stays recoverable). **Fixed by naming**: the doc says what the code does and keeps the RT-285 decision named. **Pinned** by `the_discard_doc_describes_what_the_body_does` — mutation-proved.
- **iter-1972 — the cron rewrite's atomicity note was false — a mutex is not a transaction (RT-388).** `rewrite_skill_refs`' note promised "no partial rewrites on failure" while the loop issued one `conn.execute` per job with no transaction, so a mid-loop failure left earlier jobs rewritten and later ones untouched. **Fixed**: the loop runs inside ONE explicit transaction. **Pinned** by `a_failed_mid_loop_rewrite_rolls_back_the_whole_fleet` (behavioural) — mutation-proved (the mutant showed job-1 already rewritten).
- **iter-1971 — the cron schedule self-heal's persist was discarded (RT-387).** The comment promises "the normalized form is persisted once, then used for scheduling", but the write was `let _ = update_job(…)` — a failed write left the row on its legacy 5-field schedule forever and the FIRED schedule diverged from the STORED one with no log. **Fixed**: the write reports with a warn naming both sides. **Pinned** by `the_cron_self_heal_reports_a_failed_persist` (source contract, limitation named) — mutation-proved.
- **iter-1970 — the curator's "reactivated" counter counted a change that never happened (RT-386).** `apply_automatic_transitions`' defensive arm incremented the counter and did nothing else, so the aggregate log reported a reactivation that never happened while the function's doc says the counter "describes what changed". **Fixed by naming**: the arm records at debug; the counter only counts real changes; the reactivation itself is a product decision. **Pinned** by `an_archived_skill_that_was_used_again_is_not_counted_as_reactivated` (behavioural) — mutation-proved.
- **iter-1969 — two curator archive paths dropped the tracker write that makes the archive durable (RT-385).** The file itself names the invariant at :539 ("a dropped write would leave the record Active while SKILL.md sits in `.archive` — the next pass then tries to archive a skill that is gone") and the apply path propagates with `?`; `run_review` and `run_consolidation` used `let _ =`. **Fixed**: both arms report the failure naming the re-try consequence. **Pinned** by `the_archive_paths_report_a_dropped_tracker_write` (source contract, limitation named) — mutation-proved.
- **iter-1967 — the curator's "pre-archive" backup was taken after the archival it was named for (RT-384, renumbered from RT-377 after the peer took it).** `run_review`'s doc promises "Creates a backup before any archiving when not in dry-run mode", but the archive loop moved the skills first and the backup was created after, so the snapshot could not roll back the archival. **Fixed**: the backup is taken before the loop, and a failed backup means the archival does not run at all. **Pinned** by `a_pre_archive_backup_is_taken_before_the_archival` (behavioural, both halves) — mutation-proved.
- **iter-1965 — the seat-budget docs said mid-flight enforcement was still a follow-up — it has shipped (RT-376).** Two docs (`org/seat_budgets.rs` and the operator-facing `[bylaws].budget` doc) claimed mid-flight enforcement is "a recorded follow-up, not this contract", while the agent stops a token-basis turn mid-flight once the hard cap is crossed (`agent/run.rs`), scoped per-turn by the gateway runner's envelope. **Fixed**: both docs name the shipped path and what remains a follow-up (the usd-basis per-call catalog cost). **Pinned** by `the_budget_doc_names_what_mid_flight_enforcement_really_does` (covering BOTH docs) — mutation-proved.
- **iter-1964 — three org/tool docs named seams and callers that do not exist (RT-373/374/375).** (1) `worklog.rs` + `org/mod.rs` claimed `UsageAccumulator` "feeds `result.usage`, read by … the gateway's usage meter" — no `result.usage` field exists anywhere in the tree; the real consumer is the cron scheduler's `drain_run_usage`. (2) `KaizenProposal`'s doc asserted a framework session-end caller — repo-wide grep finds none; the type is dead with a false provenance story. (3) `tool_backend_helpers`' ponytail comment grounded a static claim in `tools.web.preferred_provider = "sourcehound"` — a RETIRED migration key defaulting to `None`. **Pinned** by two contracts, both mutation-proved (MEASURED: three vacuous-needle traps — a doubled backtick, a bad split point producing `preferred_pprovider`, and a doc history note that quoted the old claim — all repaired before the proofs counted).
- **iter-1963 — the context-injection header still described three retired sources as the live phase-1 pipeline (RT-372).** The header described `Self`/`Global`/`Dept` as live org-internal sources reading the worklog while the collector returns `Ok(Vec::new())` for all three ("retired to Holon", G+1) and `crate::org`'s own module doc already records the retirement — the two docs contradicted, and a maintainer believed three feeds were injected when only Dm/Feed can produce an item. **Fixed**: the header names the retirement and what is live; the ponytail and injector lines now agree with the collector. **Pinned** by `the_injection_header_names_the_retired_classes` — mutation-proved.
- **iter-1962 — the discord schema string repeated the blanket "default 50" that RT-352 retracted from the struct doc (RT-371).** The tool's schema is built two ways and the hand-written SCHEMA STRING was the second copy: the model-visible wire contract still promised 50 from `search_members` while the action resolves 20 (only `fetch_messages` defaults to 50). **Fixed**: the string states both defaults with each action named; the RT-352 contract now pins BOTH copies. **Pinned** by the extended `the_limit_doc_states_each_actions_real_default` — mutation-proved (the first mutation attempt left a dangling literal and was discarded as a compile error, not a proof).
- **iter-1958 — the breaker's `Auth` class doc named only the credential family the router sends it (RT-369).** `classify_error` routes TWO target faults into the same class — "chat not found" and "bot was blocked" — while the doc read "Credentials are dead: 401/403/… The fix is operator action", so an operator was sent to the credential when the fault was the target (a wrong channel id, a deleted chat, a bot removed/banned). **Fixed at the doc** (the routing is unchanged: both families are non-retryable operator-action faults; re-classifying would change cooldown behavior — a decision, not a doc fix): the class doc now names both families and each one's fix. **Pinned** by `the_auth_class_doc_names_both_families_the_router_sends_it` — mutation-proved (and MEASURED: the first cut's needles matched the ROUTER's marker list and passed on the mutant; the needles are now doc-only phrases).
- **iter-1956 — the turn-exit classifier's docs claimed a journal column and live branching that do not exist (RT-368).** The module doc claimed `TurnExitReason` + `classify_turn_error` are "the machine-readable exit classification written to the turn-state journal at close (the exit_code/exit_reason columns)" and the enum doc claimed "retries, breakers, and audits branch on THIS". Neither is true: `classify_turn_error` has ZERO production callers, and the live classification is a different enum (`crate::agent::TurnExitReason`, built by `turn_finalizer`). **Fixed by naming**: both docs state the surface is UNWIRED and name the live path. **Pinned** by `the_turn_exit_classifier_names_its_unwired_state` — mutation-proved.
- **iter-1954 — a failed skill-use telemetry write left the curator under-counting the skill (RT-367, renumbered from RT-366 after the peer's iter-1932 took it).** `record_skill_use` discarded the telemetry write's `Result`. The `with_exclusive_lock` protects a CONCURRENT record; it does not protect a FAILED one — a failed load/save inside the lock left the curator reading telemetry that silently under-counts the skill. **Fixed**: the write reports with a warn naming the under-count. **Pinned** by `the_skill_use_telemetry_reports_a_failed_write` — mutation-proved.
- **iter-1953 — a failed evolution-counter persist left the review intervals hydrating stale counts (RT-365).** Both counter-persist sites (memory-review and skill-nudge) did `let _ = self.database.set_session_metadata(...)` while their comments state the persist exists "so the next run() can hydrate" — a dropped write hydrated STALE counts into the next run and the interval fired at the wrong cadence (or never) with nothing logged. Same class as rewind's transcript write. **Fixed**: both writes report with a warn naming the consequence. **Pinned** by `the_counter_persists_report_a_failed_write_instead_of_dropping_it` — mutation-proved.
- **iter-1950 — repair of three round-owned regressions found by the end-of-round verification sweep (RT-364).** The seat_budgets RT-340 contract was VACUOUS since iter-1928 (its doc comment quoted the swallow shape the assert said must be absent); `gateway/admin.rs`'s test module tripped `items_after_test_module`; my RT-352/RT-353 test imports were unused and the unit-struct default tripped a lint; and iter-1891's stale doc block in `web_tools.rs` tripped `empty_line_after_doc_comments`. All repaired — tests green, clippy gate clean.
- **iter-1949 — the @-reference doc advertised two spellings the parser resolves into broken GitLogs (RT-363).** The module doc listed `@git:diff` / `@git:staged` / `@git:N`. The parser has no `@git:diff`: it reads `@git:<anything>` as the LOG form, so `@git:diff` became a GitLog whose target is the literal word "diff" — `git log -diff -p`, which no git accepts. The real spellings are the bare `@diff` / `@staged` (what the CLI suggests). **Fixed**: the doc lists the real spellings and states the old ones resolve to broken GitLogs. **Pinned** by `the_reference_doc_spells_the_forms_the_parser_accepts` — mutation-proved.
- **iter-1948 — `set_secret_perms` WIDENED a 0o400 secret store to 0o600 (RT-361).** The doc promised "if the file already exists at a tighter mode (e.g. 0o400), leaves it. If looser, tightens to 0o600" — and the condition was `current != 0o600`, which rewrites ANY mode that is not exactly 0o600, including a tighter 0o400: a silent permission regression on a secret store. **Fixed**: tighten-only (`current & !0o600 != 0`). **Pinned** by `set_secret_perms_tightens_but_never_widens` — mutation-proved. Also from the same sweep: the `browser_tool` and `credential_pool` items were verified FALSE first-hand (`accessibility_tree` IS handled; the EXHAUSTED_TTL constants ARE read) and are dropped as residuals.
- **iter-1947 — the rate limiter's doc claimed gateway status querying that nothing does (RT-360).** The module doc advertises "status querying for the gateway", but `get_status` and `RateLimitStatus` have zero callers outside the module — no gateway command surfaces a bucket's remaining tokens. The retry/backoff half IS live (`client.rs` drives `try_acquire` and `exponential_backoff_secs`), so this is a half-wired surface, not a dead module. **Fixed by naming (the RT-275 precedent, applied as PARTIAL)**: both docs state the unwired half; exposing a status command is named as a product decision. **Pinned** by `the_rate_limit_status_names_its_unwired_half` — mutation-proved.
- **iter-1946 — the persistence seam documented a mounting that does not exist, and the mounting is not a drop-in (RT-359).** `PersistenceSeam` (the Rust prompt/subagent → Python sidecar file bridge) has no reference anywhere outside its own module, while its doc describes a two-instance mounting. Mounting as the doc says is NOT safe: `HarnessHost::add_seam` keys seams by `name()` and `PromptSlotSeam` already owns the "prompt" kind (the C1 slot seam), so `PersistenceSeam::prompt()` would EVICT it. **Fixed by naming**: both docs state the seam is UNWIRED and name the collision as the concrete blocker; folding the persistence into the slot seam (or renaming this kind) is a design decision. **Pinned** by `the_persistence_seam_names_its_unwired_state` — mutation-proved.
- **iter-1945 — the LCM tools' module header promised a registration that never happens (RT-358).** The header said that when `agent.context_engine = "lcm"` its two tools are registered so the model can use the lossless store directly. Nothing in the workspace mounts ANY of the module's 16 tool structs — the only external references to the tool names are icon and parameter strings in `gateway_runner` and the CLI's command docs. **Fixed by naming (the RT-275/RT-331/RT-332/RT-345 precedent)**: the header states the surface is UNWIRED and that mounting the tools is a product decision. **Pinned** by `the_lcm_tools_name_their_unregistered_state` — mutation-proved.
- **iter-1944 — `file_search`'s dead "Invalid regex pattern" arm advertised regex semantics the tool does not have (RT-357).** The pattern is escaped before the regex build, so the match is LITERAL and the build's `Err` arm cannot fire for anything a caller can type (an escaped literal always compiles; the only remaining failure is the engine's compiled-size limit). The message told the caller their query was a bad regex when it can never be one. **Fixed**: the message names the real cause (the size limit) and the literal semantics are pinned. **Pinned** by `file_search_matches_the_pattern_literally_not_as_a_regex` — mutation-proved (the escape removed → "a.b" also matches "axb").
- **iter-1943 — the checkpoint registration seam logged a load that never happened (RT-356).** `register_checkpoint_tool` constructs a `CheckpointTool`, drops it, and logged "Checkpoint tool loaded" — a success line for a load that never happened, in a function with zero callers anywhere in the workspace (the real registration mounts `CheckpointTool` directly through the registry). **Fixed (the RT-275 naming precedent)**: the seam's doc states it is UNWIRED AND INERT and the log line now says what actually happens. **Pinned** by `the_registration_seam_names_its_inert_state` — mutation-proved (and MEASURED: both the test's own doc comment and the production doc had to stop quoting the old log text before the contract could discriminate).
- **iter-1942 — the ACP `session/cancel` notification DESTROYED the session (RT-355).** The ACP contract cancels the in-flight prompt, not the session: the client keeps prompting. The dispatch arm called `sessions.end(...)` — the same removal the teardown path uses — so a client that cancelled one prompt had its session destroyed and every later prompt rejected as unknown. The old test pinned the defect. **Fixed**: cancel acknowledges the notification and preserves the session. **Named, not shipped (RT-275 precedent)**: the in-flight prompt is still not interrupted (no abort handle), and the limitations section says so plainly. **Pinned** by `session_cancel_produces_no_response_and_keeps_the_session` — mutation-proved.
- **iter-1941 — the tool-result anomaly budget was per-CONVERSATION, so one bad turn disabled it for the session's life (RT-354).** `count_injected_retries` scanned the whole message history, and the injected markers live in `self.messages` (a persistent session's history), so a session that spent its three anomaly retries in an earlier turn got NO anomaly retry for the rest of its life — the recovery path silently stopped recovering exactly where the budget doc says it applies (per turn). **Fixed**: the count is per turn, bounded by the last user message that is not itself an injected marker (the naive "after the last user message" window does not work — the markers are pushed as USER messages). **Pinned** by `the_anomaly_budget_is_per_turn_not_per_conversation` — mutation-proved.
- **iter-1940 — two tools reported themselves available for a credential the runtime never reads (RT-353).** `homeassistant`'s `is_available` accepted EITHER `HASS_TOKEN` or `HASS_URL` while HASS_URL DEFAULTS to homeassistant.local — a URL-only setup looked available and every call 401'd. `discord`'s (the tool AND the admin path) accepted `DISCORD_TOKEN` as well as `DISCORD_BOT_TOKEN` while the runtime reads `DISCORD_BOT_TOKEN` only — a legacy-token setup looked available and every call failed with "not configured". **Fixed**: both gates require the credential the runtime reads. **Pinned** by two env-locked tests — both mutation-proved.
- **iter-1939 — four tool surfaces that disagreed with their own schemas and docs (RT-352).** (1) `notify`'s `priority` knob was deserialized and never read — retracted. (2) `discord`'s shared `limit` doc claimed "default 50" for BOTH actions while `search_members` resolves an absent limit to 20 — the doc now states each default. (3) `memory_search`'s `max_results` doc said "default 8" while the code resolves to 10 — the doc now says 10. (4) `timestamp`'s DATE path ignored the caller's `unit` — `parse_date` returns SECONDS, so a `unit: "ms"` request returned the seconds value LABELED ms, a 1000× mislabel against the no-date path beside it — the date path now converts through `to_unit()`. **Pinned** by four tests, all mutation-proved in one run. (The `tool_search`/deferred threshold item from the same sweep is NOT included: the claimed site could not be verified first-hand and stays a named residual.)
- **iter-1938 — every hook saw its event as the literal "test" (RT-350).** The runner exported `OPERANT_HOOK_EVENT=test` on EVERY path: the registry dispatch (which has the live `HookEvent` in hand) and the CLI's `hooks test` (which has the hook's own event on disk). A hook that branches on `$OPERANT_HOOK_EVENT` ran the WRONG branch in production, and an operator testing a SessionStart hook silently tested the wrong branch too. **Fixed**: both call sites pass the real event (the registry resolves the live event through `event_name()`, the CLI passes the stored event, no event still falls back to "test"). **Pinned** by `a_hook_sees_the_event_it_is_actually_running_for` — mutation-proved. Also re-repairs RT-340's skills_guard contract, which an earlier restore-from-backup had reverted to its self-satisfying form.
- **iter-1937 — a CLI roster re-admission clobbered the binary's real version probe (RT-349).** `admit_with_capabilities` wrote `version_args: vec!["--version"]` on EVERY insert, so re-admitting a binary replaced the probe its earlier discovery had learned (kubectl wants `version --client --short`) with a flag the binary does not accept — the next version check failed on a CLI that had worked the week before. **Fixed**: a re-admission preserves the previous `version_args` (a new admission still defaults to `--version`); the capabilities field had always carried this shape. **Pinned** by `a_re_admission_preserves_the_binarys_real_probe` — mutation-proved.
- **iter-1936 — every checkpoint list row reported 0 files changed / 0 insertions / 0 deletions (RT-348).** `list_checkpoints` built each row with the diff stats hardcoded to zero while the UI presents them as the row's content — the one thing a checkpoint list exists to show (what each checkpoint changed) always read "nothing changed". `git log --shortstat` has carried the real numbers all along. **Fixed**: the log runs with `--shortstat` and the stat line attaches to the commit it follows (a subject containing `|` is not mistaken for a commit line because a hash must be 40 hex chars; git's singular forms parse). **Pinned** by `a_listed_checkpoint_reports_the_real_diff_stat` — mutation-proved. **Process note**: the first mutation run appended `git checkout --` as a restore fallback and it WIPED the fix and the test (the exact hazard the campaign notes name); the pre-mutation backup restored both.
- **iter-1935 — the process tool advertised a notify-on-complete knob with no notification path behind it (RT-347).** `notifyOnComplete` was deserialized and never read at EITHER layer — the tool never passed it to the registry, and the registry's `ProcessSession::notify_on_complete` is initialized false and read by nothing (there is no notification path from a completed background process to a turn). **Fixed**: the tool arg is retracted (the RT-344 precedent) and the registry field is NAMED as stored-never-read (the RT-275 precedent) — wiring the two together would only fabricate a notification. **Pinned** by `the_schema_does_not_advertise_a_notification_knob_with_no_notification` — mutation-proved.
- **iter-1934 — `file_state` could not hash a directory: every check/watch/diff failed at the LAST step (RT-346).** `get_file_metadata` records directories (the "/d" permission suffix, `fs::metadata` accepts them) but the hash step used `fs::read`, which fails with "Is a directory" — so the operation failed after the code had described the directory. **Fixed**: a directory hashes as the sorted sequence of its files' relative paths and bytes (an empty directory gets a stable `dir:` marker). MEASURED while building: the first cut collected RELATIVE paths and fed them to both the digest and `fs::read` — the reads silently failed against the process cwd and the hash covered NAMES only; the regression test caught it, so the collect keeps absolute paths and the digest takes the relative spelling. **Pinned** by `a_directory_hashes_deterministically_and_reports_content_changes` — mutation-proved.
- **iter-1933 — the runtime adapter trait claimed an orchestration query that does not exist (RT-345).** `RuntimeAdapter` (module header + trait doc) claimed "the orchestration loop queries these capabilities to adapt its behavior — for example, disabling tool execution on runtimes without shell access". Nothing calls the trait, its three implementations, or the two `lib.rs` re-exports; `operant-api::runtime_traits` is the live twin. **Fixed by naming (the RT-275/RT-323/RT-331/RT-332/RT-340 precedent)**: both docs state the surface is UNWIRED and that deleting the module and its re-exports is a product decision — an extension surface is not deleted by a silent refactor. **Pinned** by `the_runtime_adapter_names_its_unwired_state` — mutation-proved.
- **iter-1932 — `video_analyze`'s `analysis_type` knob was deserialized and never read (RT-344).** The schema advertised it (default "describe") and the request body carried only `video_url` and `prompt` — the knob silently did nothing, as the stale comment on the request body admitted. **Fixed by retracting the knob** (the endpoint's real parameter name cannot be verified offline, and guessing one invents a silent API contract): the field, its default fn, and its orphaned test are gone. The retraction note deliberately does not spell the knob's name — this struct's doc comment IS the schema's description, so spelling it would put the dead setting back in the schema the agent reads. **Pinned** by `the_schema_does_not_advertise_a_knob_no_request_reads` — mutation-proved (the first needle was snake_case and could not fail; the schema serializes camelCase).
- **iter-1931 — `learning_manage`'s edit reported the NEW contents as the "previous" snapshot (RT-343).** The read-before-edit snapshot (hermes-parity C4: surface the node's current contents so the agent sees exactly what it is about to change) ran AFTER the edit, because the match arm performs `edit_node` first — so `previous.body` held the NEW content and the payload asserted the file already looks the way the agent just made it look. **Fixed**: capture the snapshot BEFORE the mutation runs, with the order's rationale in the comment. **Pinned** by `an_edit_reports_the_previous_contents_not_the_new_ones` (which also asserts the node really changed, so the snapshot is the previous state and not a stale read) — mutation-proved.
- **iter-1930 — a webhook route claimed by two subscriptions resolved NONDETERMINISTICALLY (RT-342).** `subscription_for_route` took the first match from a HashMap's iteration order, and the store is re-read (into a fresh, randomly seeded HashMap) on EVERY request — so with two subscriptions claiming one route the winner varied per request and across restarts: the same external event could run a different prompt against a different endpoint each time. **Fixed**: sort by the map key (the endpoint URL) before the find. **Pinned** by `a_claimed_route_resolves_deterministically_when_two_subscriptions_claim_it` — 20 lookups (each a fresh randomized HashMap) — the mutation run failed 3/3 executions.
- **iter-1929 — the env secret probe matched patterns as SUBSTRINGS, so `TOKENIZERS_PARALLELISM` read as a secret (RT-341).** `looks_secret` used `lower.contains(pattern)`, so `auth` matched `MY_AUTHORIZATION_LEVEL` and `token` matched `TOKENIZERS_PARALLELISM`. Both harmless individually — but a report that cries wolf on the HF tokenizers flag teaches the operator to ignore the report, which is the tool's whole value. **Fixed**: match at an ENV-NAME SEGMENT boundary (non-alphanumeric on both sides), so `openai_api_key` still hits `api_key` (the documented case, underscore significant) while `authorization`/`tokenizers` no longer hit the bare patterns. **Pinned** by `a_secret_name_matches_at_a_segment_boundary_only` — mutation-proved.
- **iter-1928 — three seat/auth surfaces whose docs promised more than the code does (RT-340).** (1) `seat_budgets`' migration marker writes were `let _ = conn.execute(...)` — silent against the doc's own "best-effort: a failure here is reported (warn)" contract, so a failed marker re-ran the copy at every boot with no trace (both writes now warn). (2) `seat_policy`'s `DelegationPosture` variants document enforcement ("every `delegate` call is denied") that the retired consult path never performed — the posture is stored, the tool-config gates own every verdict (labeled STORED, NOT ENFORCED). (3) `skills_guard::content_hash` truncates the SHA-256 digest to 8 bytes / 16 hex chars while the label read as a full SHA-256 (the label now names the truncation; nothing verifies a skill against it, an RT-275-class dead surface). **Pinned** by three source contracts — the skills_guard one caught ITSELF satisfying its own comment during the mutation run (the same trap class as the empty-suffix split) and was reworded — all mutation-proved.
- **iter-1927 — the compact accessibility render took a heading level's CHARACTER COUNT as the level (RT-339).** `render_node_compact` repeated `#` by `level.len()` — but `level` is a DOM property value (arbitrary text from a real page's ARIA tree), so level "12" printed TWELVE hashes and "abc" printed three. The render is the model's sole view of page structure, so a malformed heading degrades it silently. **Fixed**: parse the level, clamp to the HTML heading range (1..=6), fall back to h1 for anything unparseable — the HTTP-level value, not its byte length. **Pinned** by `a_heading_level_outside_the_html_range_render_as_h1` — mutation-proved.
- **iter-1926 — the cookie jar wrote HttpOnly into the netscape format's SUBDOMAIN column (RT-338).** The format's second column is the include-subdomains flag (carried as a leading dot on the domain); `to_netscape_line` wrote HttpOnly there, so a host-only insecure cookie exported as a GLOBAL and secure one — and HttpOnly, which has no column in the format at all, was dropped. The parser had the mirror-image bug (it read column 2 back as HttpOnly), so our own files round-tripped "fine" while every file from a real exporter was misread. **Fixed on both sides**: the writer derives the flag from the original domain's dot and carries HttpOnly via curl's `#HttpOnly_` prefix; the parser strips that prefix before its comment skip and restores the dot when the flag is TRUE. `cookies_to_netscape` (the `operant cookies export` path) emits the prefixed line. **Pinned** by `a_netscape_round_trip_keeps_the_flags_in_their_real_columns` — including the discriminating case (HttpOnly with a dotless domain must NOT set the subdomain flag; the first attempt could not fail because both conditions were true for its cookie) — mutation-proved.
- **iter-1925 — the `kernel_exec` cell timeout could read as UNBOUNDED on a zero-second setting (RT-337).** `cell_timeout_secs` was `settings.request_timeout_secs.saturating_sub(1)`, so a 0 produced 0 — and 0 to the sidecar means UNBOUNDED. The same shape was fixed for `code_execution` in iter-1923 (RT-335), whose ledger row named this site; this is that canonical floor, documented inline with the head-start rationale.
- **iter-1923 — two advertised timeout params were never read: `code_execution timeout=0` ran UNBOUNDED and `browser_cdp` had no bound at all (RT-335).** (1) The kernel cell timeout used `timeout.as_secs().saturating_sub(1)`, so a 0-second arg produced `cell_timeout_secs: 0` — and 0 to the sidecar means UNBOUNDED: a 2s timeout that produced a clean sidecar timeout error let an infinite loop run until the outer request cap. The identical shape sits in `kernel_exec` (its own request cap masks it there). (2) `browser_cdp` advertised `timeout` since the tool shipped and nothing read it — a hung CDP socket hung the turn with no bound. **Fixed**: a documented floor (`kernel_cell_timeout_secs` — the one-second head-start is retained, one second is the minimum) and `with_optional_timeout` wrapping BOTH CDP round-trip seams. **Pinned** by `a_zero_second_python_timeout_still_bounds_the_cell`, `the_timeout_arg_bounds_a_cdp_round_trip` and `the_timeout_arg_reaches_both_cdp_round_trips` — the last because the helper alone is not the fix — all mutation-proved.
- **iter-1922 — an SMTP connect failure for the standard implicit-TLS endpoint arrived opaque (RT-334).** The port rule is `host.contains(":")` → append nothing, else append `:587` — so `smtp.example.com:465` (the standard implicit-TLS form) got a plaintext TCP connection, the greeting read failed, and the operator saw a bare "SMTP connect failed: …" with no way to tell the endpoint itself (port 465) is unsupported on this path (which speaks plaintext SMTP + STARTTLS only). **Fixed by naming**: the connect error now names the supported form (port 587 submission + STARTTLS, or an explicit `host:port` whose server speaks STARTTLS) and calls out 465 as unsupported. **Pinned** by `a_failed_smtp_connect_names_the_supported_form` — an unresolvable `.invalid` host fails fast at DNS (offline, deterministic) — mutation-proven.
- **iter-1921 — the admin handler pretended to shut the gateway down, and no production path routes it at all (RT-333).** Two findings in one file: (1) every caller of `handle_admin_command` is in this crate's TEST module — no production path routes an admin command there, so the handler is real (and per-arm tested) but unrouted; (2) its `shutdown` arm replied "Shutdown initiated. Goodbye!" when nothing on the path stops the gateway — a fabricated claim, sitting beside `broadcast`'s honest "Not yet implemented". **Fixed by naming (the RT-275/RT-323/RT-331/RT-332 precedent)**: the handler's doc states it is UNWIRED and that wiring the dispatch is a product decision; the shutdown arm now names the not-enacted state and tells the operator to stop the process externally. **Pinned** by `the_shutdown_arm_does_not_claim_a_shutdown_it_never_ran` — mutation-proved.
- **iter-1920 — the response mirror was documented as a live hermes-parity feature with nothing routing it (RT-332).** `mirror_targets` has zero callers on the delivery path (only its own test) and its doc claimed hermes `gateway/mirror.py` parity — where the parity method FORWARDS the mirrored copies. Here the parity is DATA-only: rules register, targets compute, nothing sends, so no response is ever mirrored on any platform. The `with_mirror_rule` doc repeated the claim. **Fixed by naming (the RT-275/RT-323/RT-331 precedent)**: both docs now state the forwarder is unwired and that routing the copies through `send_to_platform` is a product decision. **Pinned** by `the_mirror_parity_names_its_unwired_forwarder` — mutation-proved.
- **iter-1919 — the session stall detector was presented as live machinery with nothing routing it (RT-331).** `SessionStallTracker::stalled()` has zero production callers: the lease gate itself (`TurnLease`) runs in production, but the detector that would escalate stalled sessions is complete and unread, while the struct's doc presents it as the live half of the design. **Named, not shipped (the RT-275/RT-323 precedent)**: the doc now states the detector is unwired and that wiring a sweep over `stalled()` is a product decision. **Pinned** by `the_stall_detector_names_its_unwired_state` — a source-grep contract whose needle is concatenation-built at a WORD boundary (the first attempt split it at an empty suffix and the test's own construction line satisfied it — the exact flaw the concatenation rule exists to prevent, caught by the mutation run) — mutation-proved.
- **iter-1918 — the gateway's `edit_message` bypassed the dispatch breaker entirely (RT-330).** The edit arm had no `breaker_guard` and no `breaker_record`, while the doc above the streaming send says the id-back callers ("proactive sends, edits") must not bypass the guard the plain send enforces. A tripped breaker let edits through to the platform, and edit failures never walked the breaker — so an edit loop against a dead channel never tripped the circuit that would have stopped the sends. **Fixed**: the same seam as the streaming send (guard before dispatch, record the outcome after). `delete_message_on_platform` deliberately does NOT get the guard: it carries no `OutgoingMessage` for the content-scoped breaker to check, so a guard there would be a fabricated check — it is named in the ledger row instead. **Pinned** by `edit_message_shares_the_breaker_seam` — mutation-proven (the guard out → the edit reaches the adapter and the fast-fail is gone).
- **iter-1917 — the turn summary's "+N more" overflow count was dead for one of its two sources (RT-329).** `ordered` holds every verb group, so the `ordered.len() < tally.verbs.len()` subtraction could never fire — and initializing `extra` from it MASKED the real overflow (the `take(MAX_SEGMENTS)` cap) as zero. A five-verb turn reported only the other-tool count: the user saw "+1 more" where two things were hidden. **Fixed**: the overflow is what the cap skipped (verb groups, the original unit), plus the other-tool count. **Pinned** by `a_capped_verb_list_reports_what_it_hid` — mutation-proven (the dead branch restored → "+1 more", the dropped group masked).
- **iter-1916 — three kernel-tool defects: the namespace param lost to the session id, `kernel_state {kind}` could never list a kind, and a lesson written this turn was invisible to the next turn's prompt (RT-328).** (1) `kernel_exec` read `context.get("session_id").or(args.namespace)` — the session id WON, so the model could not pin a namespace and the result's `session` field misreported which one it used (the field's doc says the namespace "defaults to the current session id"). (2) `kernel_state {kind: …}` with no id was routed to `harness_overview`, which the sidecar runs with NO kind filter — only `harness_list` takes one — so the documented "list that kind's entries" was impossible and every kind-less call returned the whole unfiltered overview. (3) `KernelRuntime::request`'s injection-cache invalidation list named `harness_set`, a method the sidecar does not have, while the real write (`harness_upsert`, the persistence seam's path) missed the invalidation — a lesson written this turn was absent from the next turn's prompt for the 5s TTL. **Fixed** all three; the args doc now names the real listing mechanism. **Pinned** by `kernel_exec_honours_an_explicit_namespace`, `kernel_state_lists_the_requested_kind` and `a_harness_upsert_invalidates_the_injection_cache` — all three mutation-proved (each defect restored → the named test fails).
- **iter-1915 — an org-context item that FIT the remaining total budget was silently dropped (RT-327).** The context injector's class loop `break`ed on the first item that exceeded `total_char_cap`, abandoning the rest of the class — and those items never reached the roll-over pool (only quota-overflow items are pooled), so the pool could not recover them. Items rank by SCORE, not length, so one long high-score item starved short items that would have fit: with the cap at 300, a 400-char DM followed by a 100-char DM rendered NEITHER, though 300 chars of budget remained and the seat never learned the second message existed. **Fixed**: `continue` — the long item skips its own slot, shorter items after it still render, and the pool loop keeps enforcing the total cap (an item that exceeds the cap here exceeds it there too). **Pinned** by `a_short_item_still_renders_when_a_longer_one_exceeds_the_total_cap` — mutation-proven (`break` back → the rendered section is EMPTY: both items lost).
- **iter-1914 — a `` `` `` (double backtick) pair ate the next character and flipped code state for the rest of every Slack/WhatsApp message (RT-326).** The fence detector in `asterisk_dialect` (Slack mrkdwn + WhatsApp ports) checked `ch == '\`' && chars.peek() == Some(&'\`') && chars.peek() == Some(&'\`')` — but `Peekable::peek()` does NOT advance, so both calls observed the SECOND backtick: the guard only proved two backticks exist and then consumed THREE items. Any message carrying a `` `` `` pair (markdown's ``code`` escape, an empty `` `` ``, or two backticks in CLI/JSON output) lost its next character, gained a spurious ``` fence, and from that point on bold/italic/strike conversion was suppressed for the remainder. No test covered a two-backtick input. **Fixed**: consume-and-check — the second backtick is consumed first, and only a THIRD backtick toggles the code-block state (a two-backtick span passes through untouched). **Alongside**: the module header no longer claims "a single public function" (there are three). **Pinned** by `asterisk_dialect_does_not_eat_the_char_after_two_backticks` — mutation-proven (the double-peek back → "use ``` for empty" comes out `use ```for empty`).
- **iter-1913 — the last two sites of the byte-slice panic class: the working-diff truncation and the skill-description cap (two listing sites) (RT-325).** `working_diff`'s `truncate` cut a real git diff with `&text[..max_bytes]` (a 100 KiB cap over diffs that carry comments, strings and unicode identifiers) and `skills_tool`'s description cap cut SKILL.md frontmatter with `&s[..MAX_DESCRIPTION_LENGTH - 3]` in TWO listing sites (an em-dash or curly quote in a description is ordinary). Both panic with "byte index N is not a char boundary" — the working-diff one inside a tool call, the skills one on every `skills list` over an affected skill. **Fixed**: the diff cut uses the crate's own `crate::agent::safe_truncate_str`; the description cap uses the `chars().take()` shape the `name` field right beside it already used. **The RT-301 guard gained its ninth and tenth cases**, so the class is now pinned across all ten files that ever carried it. **Pinned** by `diff_output_truncation_survives_a_multibyte_boundary` and `a_description_truncation_survives_a_multibyte_boundary` — both mutation-proven ("end byte index 102400 is not a char boundary; it is inside '日'" / "end byte index 1021 is not a char boundary; it is inside '—'").
- **iter-1912 — a failed OSV lookup was recorded as "no_vulnerabilities_found" — the malware gate failed OPEN (RT-324).** `check_packages` ran `query_osv(pkg).await.unwrap_or_default()`, so every network error, timeout, non-2xx response and parse failure became an EMPTY vuln list — which the next arm records as a clean package. A malware package whose lookup transiently fails reads as CLEAN to the agent deciding whether to install it. The sibling `check_package_osv` has always propagated with `?`, and execute's own Err arm already reports `"status": "error"` with the message — the honest report existed and was unreachable. **Fixed**: the query's error propagates. **Pinned** by `a_failed_osv_lookup_is_not_recorded_as_a_clean_package` (source-grep contract — the URL is hardcoded so a network test would be environmental; the contract also pins both propagation sites) — mutation-proven (the swallow back → the test fails by name).
- **iter-1911 — the file-tools' path-safety doc promised home-directory containment that the code NEVER implemented (RT-323).** The module doc has claimed since iter-125 that any canonicalized path that "lives outside the home directory" is rejected — `validate_path` implements (a) `..` traversal rejection and (c) the hard-deny list plus the credential registry, but the containment arm (b) does not exist anywhere in the tree, so `/etc/passwd` and any absolute path outside `$HOME` read and write freely while the doc says otherwise. A phantom gate is worse than a documented gap: the next reader trusts it. **Fixed honestly, not silently**: the doc now names exactly the three arms that ship (`..` traversal, the deny list, the credential registry) and records containment as NOT enforced — with the reason a strict version is not the default (it would also reject the agent's own workspace outside `$HOME`, e.g. a container's `/workspace`). Containment stays a named feature decision, the RT-198 precedent for a false safety claim. **Pinned** by `the_path_safety_doc_does_not_promise_a_gate_that_does_not_exist` (source-grep contract over the module's own doc, needle by concatenation) — mutation-proven (the phantom claim back → the test fails by name).
- **iter-1910 — an IPv4-mapped IPv6 address (`::ffff:a.b.c.d`) passed the SSRF gate as "safe" (RT-322).** `is_blocked_ip`'s V6 arm checked loopback / link-local / multicast / unspecified / ULA but never consulted the embedded IPv4 address, so `http://[::ffff:169.254.169.254]/` — the cloud-metadata range in its mapped spelling — and `http://[::ffff:127.0.0.1]/` returned `Ok(true)` on the literal-IP path. The module's own doc promises "private, loopback, link-local, CGNAT, benchmarking, and cloud-metadata ranges", and a mapped private IP is one of those addresses. **Fixed**: one arm — `v6.to_ipv4().is_some_and(|v4| is_blocked_ip(IpAddr::V4(v4)))` — the mapped form now carries its IPv4 verdict (a mapped PUBLIC address stays allowed). **Alongside**: the module header no longer claims the (zero-caller) `osv_check` half is used by `skills_guard` — the live tool is `tools::osv_check`, which does not import this module; the dead copy is named, not silently deleted (RT-275-class owner decision). **Pinned** by `ipv4_mapped_ipv6_is_blocked_like_its_ipv4_address` — mutation-proven (the arm back out → `::ffff:127.0.0.1 must be blocked`).
- **iter-1909 — the `process` tool's output trim cut with a BYTE slice, and the panic it would cause kills the reader task and wedges the child on a full pipe (RT-321).** The registry keeps only the last 200 000 bytes of a child's output and cut with `out[out.len().saturating_sub(MAX_OUTPUT_CHARS)..]` — `out` accumulates ARBITRARY command output, so a CJK build log (`yes 日本 | head -c 300000`) put the cut inside a multi-byte character and the slice panics. The panic runs inside a `tokio::spawn`ed reader whose `JoinHandle` is DISCARDED: the drain silently stops, the child blocks on the 64 KiB pipe forever, and the session completes only at the 300 s wait timeout with a partial buffer. The constant was also named `MAX_OUTPUT_CHARS` while being compared against `String::len` (bytes). **Fixed**: one shared char-safe trim (`trim_to_tail` — walk forward to the next boundary, `drain` the head) now serves both the stdout and stderr loops, and the constant is renamed `MAX_OUTPUT_BYTES` with its unit documented. The RT-301 guard gained its eighth case. **Pinned** by `the_output_trim_never_panics_on_a_multibyte_boundary` — mutation-proven (the byte slice back → "start byte index 400003 is not a char boundary; it is inside '日'").
- **iter-1908 — `bash -c"rm -rf /etc"` (the GLUED form) bypassed the blast-radius gate entirely (RT-320).** A real shell runs the glued form identically to the spaced form (getopt: the `c` flag's argument is the rest of the word — `-c"rm -rf /etc"` is ONE word after quote removal). The gate only entered its `-c` arm for tokens ENDING in `c`, and its `glued_script` helper then `rfind`'d the LAST `c` in the token — which for this shape is the one inside `/etc`, so the "glued script" came out empty, the segment became the unfollowable sentinel, `rate_segment` rated it `Scoped` (not `Unbounded`), and `screen()` found no destructive target and passed it with NO justification. **Fixed** with bash-faithful parsing: the leading ASCII-letter run is scanned for the FIRST `c`, and the script is the rest of the word from there (`-crm -rf /etc` → `rm -rf /etc`; `-lc…` the same; a cluster with no `c` falls through as before). **Pinned** by `bash_c_glued_to_the_script_is_unwrapped_like_the_spaced_form` (the classifier rates the glued destructive form `Unbounded`, the gate fires, and a glued benign script stays `None`) — mutation-proven (the `rfind` helper back → the test fails by name).
- **iter-1907 — every successful non-streaming provider response over 64 KiB was silently truncated, breaking `embeddings` for any real model (RT-319).** `execute_with_retry` read its body through `read_bounded_error_body` BEFORE looking at the status and returned that body on the success path — so the 64 KiB ERROR-body cap applied to success bodies too. One real embedding model (1536 dims) emits ≈21 KiB per vector and the engine batches 32, so the FIRST batch with ≥3 uncached candidates was cut and `serde_json::from_str::<EmbeddingsResponse>` failed with a parse error; non-streaming `chat()` bodies over 64 KiB failed the same way. `agent.context_lcm_embedding_model` (the LCM vector-recall knob) is a live configured feature over exactly this path. The streaming path already had the right shape (early return on success, bounded read for errors). **Fixed**: the success exit reads the body whole; the bounded reader stays on the error paths it exists for. **Pinned** by `a_successful_response_larger_than_the_error_body_cap_is_not_truncated` (a >64 KiB 200 body whose tail marker lives past the old cut) — mutation-proven (the bounded read back → the parse fails).
- **iter-1906 — a keyless `/connect` retarget sent the OLD provider's API key to the NEW host (RT-318).** The client documents the invariant right where it is enforced — "After a runtime retarget the configured key belongs to the OLD endpoint — never send it to the new host" — and `effective_api_key` implements it. But `build_headers`, the ONE place the Authorization header is actually written, read `api_key_override` and fell back to `config.api_key` with NO retarget check. The reachable sequence is the documented local keyless use case: `PooledModelClient::set_endpoint` (ollama / lm-studio) sets the new base URL and clears the override with `set_api_key("")`, so the fallback fired and `Authorization: Bearer <old key>` rode to the new host — a credential leak across trust boundaries, invisible because the existing regression test pinned `effective_api_key` and not the header. **Fixed**: `build_headers` now honours the same precedence — a non-empty rotation override wins, the configured key only when NOT retargeted, and no header at all when neither (the keyless case is byte-identical to before). **Pinned** by `build_headers_after_a_keyless_retarget_sends_no_authorization_header` — mutation-proven (the bypass back → the test fails naming the leak).
- **iter-1905 — the provider error body's display cut byte-sliced free text — a panic in the ERROR path (RT-317).** `sanitize_error_body` collapsed the provider's non-2xx body and cut it with `&clean[..500]`; the body is arbitrary text (decoded lossily from up to 64 KiB), so a CJK/emoji error body put byte 500 inside a multi-byte character and the slice PANICKED — replacing the error with a crash, on every non-2xx response (`classify_http_error`). Live on the path the RT-301 sweep already visited FIVE times in this crate (and iter-1901 a sixth) without covering. **Fixed**: char-safe cut, the shape `error.rs`'s own fallback has used since iter-1883. **The RT-301 guard gained its seventh case** (`client.rs` / `&clean[..500]`). **Pinned** by `sanitize_error_body_never_panics_on_a_multibyte_boundary` (byte 500 inside a 4-byte emoji) — mutation-proven (the byte slice back → the test panics).
- **iter-1904 — the `patch` tool's fuzzy fallback span forever on a multi-line find near the end of the file (RT-316).** The scan loop `while i < content_lines.len()` guarded every step with `if i + find_lines.len() <= content_lines.len()` and had no `else` — when fewer lines remained than the multi-line `find` needed, the guard's whole body was skipped, `i` never advanced and the loop spun forever: the tool never returned, the turn hung, the CPU spun. The trigger is ordinary (a 2+ line `find` matching a block that is not at EOF), and every existing test used 1-line finds. **Fixed**: the pass-through (`push` + `i += 1`) is hoisted OUT of the length guard, so the tail of the file always advances. **Pinned** by `the_fuzzy_tier_advances_past_a_multiline_match_near_the_tail` — mutation-measured: the hoist back → the loop spins (the test runs 60+ seconds until the harness kills it; the spin is synchronous, so the test's own tokio timeout cannot preempt it, which the regression comment now states honestly).
- **iter-1903 — the tool schemas advertised camelCase and the tool bodies read snake_case, so half the parameterized tools were unusable by a schema-following model (RT-315).** Tool argument structs are generated with `#[serde(rename_all = "camelCase")]`, so the schema the model reads advertises `sessionId` / `filePath` / `messageThreadId` / `entityId` — while the bodies read the snake_case field names (`args.get("session_id")`). A model that followed the schema hit "Missing required parameter" on `session_search` (its scroll mode was UNREACHABLE — it needs `around_message_id`), `home_assistant` (`entity_id`), `skills_tool` (`file_path` silently returned the whole SKILL.md instead of the referenced file), `reaction` (`message_id`), `send_message` (`message_thread_id` — the Telegram forum-topic id always dropped, so a forum send failed "chat not found") and `checkpoint` (`working_dir` fell back to the process CWD and `commit_hash` to "", so `restore` ran against the WRONG tree). **Fixed at the ONE dispatch seam**: `normalize_arg_keys` (`tools.rs`) adds a camelCase key's snake_case twin at `execute_with_timeout`, so every tool reads the spelling its schema advertises — an explicit snake key always wins, and a no-uppercase args object is byte-identical (the majority of tools never advertise camelCase). `discord` was the INVERSE of the class (a hand-written snake_case schema with `rename_all` on the struct, so `from_value` silently dropped every advertised key and every parameterized action failed its required-param check) — its rename is gone, so the struct accepts its own wire contract. **Pinned** by `a_camel_case_arg_reaches_the_snake_case_reader` (the shared seam: a snake-reading body driven through the registry with `aroundMessageId`; an explicit snake key wins; a no-uppercase object still fails its required check) and `a_schema_compliant_discord_call_keeps_its_snake_case_params` — both mutation-proven (the dispatch passthrough back / the rename back → each fails by name).
- **iter-1902 — a Slack app-level token (`xapp-…`) bypassed the ENTIRE redaction pipeline (RT-314).** The pre-screen `needs_redaction` is a needle-set short-circuit whose own doc states the invariant "the needle set is a *superset* of every trigger the regexes can fire on" — but the needle set (60 entries) covered `xox` (Slack bot/user tokens) and not `xapp` (Slack app-level tokens), while `patterns()`'s token-prefix regex has carried `xapp-\d+-[A-Za-z0-9-]{10,}` all along. `redact_sensitive_text` early-returns on a needle miss, so a Slack app-level token in a tool output, an API error body, a memory/turn text or a transcript was emitted VERBATIM to the model, the logs and the transcript — the exact class the module exists to prevent. **Fixed**: one needle (`"xapp"`) — the array's documented superset is whole again. **Pinned** two ways: `a_slack_app_level_token_is_redacted` (the behavior, built by concatenation so the repo's own literal scrubber cannot weaken it) and `every_token_prefix_has_a_prescreen_needle` — a structural contract that parses this file's own two arrays and asserts every token prefix in the regex has a needle that is a prefix of it, so the class cannot return for ANY future prefix — both mutation-proven (the needle to a non-covering value → both fail by name).
- **iter-1901 — the advisory view's tool-result preview cut free text with a BYTE slice (RT-313).** `moa.rs::truncate_tool_result` built the head+tail preview with `&text[..half]` and `&text[text.len() - half..]`, and `text` is arbitrary tool output (file reads, git logs, web pages carry emoji, CJK and accented text) — so byte 2000 (and byte `len - 2000`) is not a char boundary whenever a multi-byte character straddles it and the slice PANICS. RT-301 (iter-1883) fixed five sites of exactly this class and pinned them with a source-grep contract over five files; `moa.rs` was not among them, which is why it survived to iter-1900. The panic lands inside a `tokio::spawn`ed task whose `JoinHandle` is discarded, so the gateway's idle-recap loop (attached unconditionally, `recap_idle_seconds` default 240) and the advisor path die SILENTLY: no crash report, no note — just "recap never arrives". **Fixed**: the head cut uses the crate's own `crate::agent::safe_truncate_str`; the tail cut walks forward to the next boundary (`text.len()` is always one, so the walk terminates). The RT-301 guard gained its sixth case (`moa.rs`), so the class cannot return to this file. **Pinned** by `advisory_view_survives_a_multibyte_char_at_the_truncation_boundary` ("日".repeat(4000) = 12,000 bytes, both cuts mid-character — the pre-existing ASCII test passes on the defective code, this one cannot) — mutation-proven (the byte slice back → the test panics with "end byte index 2000 is not a char boundary; it is inside '日'" — the defect itself).
- **iter-1900 — the `send_message` chunker panicked on a multi-byte character (RT-312).** It searched for a split boundary by slicing backwards from a char boundary (`&s[byte_off - 1..byte_off]`), but `byte_off - 1` is NOT a char boundary when the preceding character is multi-byte ('é' is 2 bytes, '🚀' is 4) — so the slice panicked with "start byte index N is not a char boundary" and took the turn with it. The trigger is ordinary: a message over the platform limit (Telegram >4096, Discord >2000, Slack >40000) whose split region contains a multi-byte character. **Fixed**: the separators are ASCII, so the BYTES are compared instead of slicing — no copy, no panic, exact. **Pinned** by `chunk_message_does_not_panic_when_a_multibyte_char_precedes_a_boundary` — mutation-proven (the byte slice back → the test panics with the defect's own message).

- **iter-1891 — `web_fetch` read the whole response body into memory (RT-306).** Its sibling `http_request` reads through a 2 MiB chunked cap — the fix R48-7 was filed for — but `web_fetch` called `response.text().await` and kept the whole body, one large (or hostile) endpoint away from taking the agent's process down. It also drifted on the method: the sibling uppercases it, `web_fetch` matched the raw string, so `{"method":"post"}` was rejected. **Fixed**: `web_fetch` uses the shared bounded reader and normalises the method case exactly like its sibling. **Pinned** by `web_fetch_reads_its_body_through_the_bounded_reader` — mutation-proven.

- **iter-1894 — `config_manage show` leaked the SMTP password (RT-307).** The tool's doc says it dumps the effective configuration "with secrets masked", but the mask rule covered `api_key` / `token` / `secret` / `password` / `auth` — and the config's field is `email_smtp_pass`, which matches none of them, so the credential was returned verbatim to the model and into the transcript. **Fixed**: the mask rule also covers `pass`. **Pinned** by `redact_masks_email_smtp_pass` — mutation-proven.

- [\[Unreleased\]](#unreleased)
- [\[0.3.0\] - 2026-10-26](#030---2026-10-26)
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

### Added

- **iter-1730 — the creator mode (gap G7 of the plugin-efficacy plan).** `operant plugins new <name> --capability <tool|observer|memory|skill>` scaffolds a plugin project in the CWD (manifest.toml + the capability's export-contract README + an extism-pdk `src/lib.rs` stub for WASM capabilities / a SKILL.md for skills) and `operant plugins validate <dir>` validates any plugin directory with the installer's own manifest parser — one authority, so a scaffold is accepted as-written and an edited one is checked before install. docs/library/plugins.md gains the "Creating a plugin" loop.
- **iter-1732 — the in-tree WASM fixture (gap G8): the observe/tool contracts proven against a REAL module.** `crates/operant-plugins/fixtures/fixture.wasm` (a 260KB extism-pdk module; source in `fixtures/fixture-plugin/`, rebuilt by `scripts/build-plugin-fixture.sh`) exports `tool_metadata` + `execute` (echo) + `observe` (allow by default, block on tool name or envelope `config.block_all`), and the new integration target `tests/fixture_plugin.rs` exercises execute roundtrip, metadata read, both block paths and config passthrough on real WASM — the contract pins were serde-only before this. The artifact is committed so tests run without the wasm32-wasip1 target.
- **iter-1733 — live plugin re-registration (gap G6): the watcher now feeds the agent surfaces, not only the kernel.** `spawn_wasm_watcher` gains a sharing handle on the agent's `ToolRegistry` (Arc-field-backed — a clone IS the live registry), and its callback calls `apply_plugin_changes` (plugin_tools.rs) alongside the kernel mount/replace: a tool plugin replaces its own registration (`register_dyn` swaps under the same name atomically; a name the plugin does not already own is still refused — KB-11 posture preserved), a changed observer re-keys by plugin name (new named `plugin_interceptors` storage in ToolRegistry with replace semantics — re-registration never stacks), and names a plugin stops providing are dropped. Install-while-running is live for tools and observers; skill roots remain restart-bound (SkillManager is boot-scanned and owned — the named upgrade). Five integration tests drive the real fixture module through the real registry dispatch (register live, no-stack re-apply, the module's Block verdict stopping a tool, disabled inertness, capability-drop removing the tool).
- **iter-1734 — `operant plugins status` (gap G9): the operator's per-plugin discovery view.** Name, version, capabilities, enabled state, signature mode, wasm module presence/size, and the tool name each tool-capable module would register (probed from its own `tool_metadata` export — one throwaway instance, the cost discovery already pays). Honest scope: it reports what the invoking process can know; the running gateway's live registry state (registered tools, active interceptors) is named as living in that process, not fabricated.

- **iter-1739 — the post-execute Replace verdict (gap G2): observers can now rewrite a tool result.** `ObserveVerdict` gains `Replace { content }` (`{"action":"replace","content":"..."}`) and `PipelineAction` gains the matching variant (the named upgrade trigger from the first port): at post-execute an interceptor's Replace overwrites the tool result's content and later handlers observe the replaced value (waterfall semantics — the redaction/enrichment seat dsh has and operant lacked); at pre-execute a Replace passes through. The message pipeline treats Replace as pass-through (tool-seam semantics). The in-tree fixture module gained a `config.replace_content` path and the chain is pinned on REAL WASM at three levels: serde pin, real-module verdict, registry-level dispatch. (This entry landed late — the iter-1739 commit staged CHANGELOG.md before its edit was written; recorded here in iter-1742.)
- **iter-1740/1742 — the turn-level pre-step seam (gap G1): a policy observer can veto or rewrite a WHOLE turn.** dsh's `agent/pre-step` veto/mutate (audit S1) is ported, collapsed to once per turn: `ToolRegistry` gains named `turn_interceptors` (replace semantics) and `consult_turn_interceptors(query)` — the first decisive verdict wins (serial waterfall collapsed); the agent loop consults it BEFORE the context prologue, so a Block verdict ends the turn with "Turn vetoed by policy: …" and the query never persists, while a Replace verdict rewrites the query the turn runs on. Opt-in via the observer manifest's `[config] turn_gate = true` (a result redactor must not silently start vetoing turns). Envelope point: `agent/pre-step`. Pinned at four levels: registry consult tests, an agent-level test proving the veto returns Ok before any model call, the REAL fixture module's turn veto (`config.block_query_contains`), and the watcher-bridge end-to-end.
- **iter-1743 — the dashboard plugin route (gap G10) + the four recorded decisions (G3/G4/G5/G11).** `GET /api/plugins` on the existing four-route dashboard server returns the plugin inventory as JSON (same discovery view as `operant plugins status`: capabilities, enabled state, wasm presence, the tool name each module's `tool_metadata` would register), behind the session token, with the live-registry honesty note in the payload. The first-party page deliberately does NOT fetch the new route — the RT-44 contract test pins the page's fetch set, which is exactly the defect the route avoids recreating. Decisions recorded in docs/library/plugins.md: G4 channel capability descoped (compile-time registry, no consumer), G5 free-form config stands (schema-free until a second consumer), G3 ask escalation deferred (Block covers today's need), G11 inventory marginal (no second publisher). The full dsh web UI stays opt-in (import plan Phase 5).
- **iter-1726 — the service half of dsh's provide/inject composition, and one policy bypass closed (audit F2 + F4).** `PluginManifest.provides` declares service names a plugin offers, and a `requires` entry now matches either another plugin's name OR a provided service name (`host.rs` fixpoint) — deepseek-harness composes on service names (`ctx.provide('memory.backend')` then `inject: ['memory.backend']`); iter-1721's collapse to plugin-name matching meant two plugins could not compose over a seam at all. Still a discovery filter (no fibers, still transitive, cycles fine) and the labels are composition-only — the host never resolves them into runtime objects. Separately, `operant plugins list` built its host WITHOUT signature policy (`PluginHost::new`), so under strict it could advertise plugins the runtime refuses to load; the list path now applies the same `with_security` policy as the loader (pinned: strict + empty trusted keys lists nothing, disabled lists the plugin).

- **iter-1725 — the red-team pass over the plugin import closed the residency gap (audit F1 + F3, `docs/audit/2026-11-05-red-team-the-plugin-architecture-import-against-deepseek-har.md`).** Every plugin invocation used to recompile the WASM module: `WasmTool::execute` instantiated a fresh `extism::Plugin` per tool call behind a comment claiming "Extism Plugin is !Send" (false for the pinned extism 1.41.0 — upstream `unsafe impl Send/Sync for Plugin`), and observers paid one load PER TOOL CALL PER POINT. Both bridges now run on resident `runtime::PluginPool`s — extism's own `Pool` (instances created lazily up to `available_parallelism`, reused across calls, 10 s wait ceiling surfaced as pool-busy), one WASM compile per module per process, deepseek-harness residency semantics without fibers. Observers also gained the manifest `[config]` passthrough: a plugin's free-form config table rides every observe envelope as `"config"` (dsh's policy plugins are all config-driven; without it a policy observer has nowhere to read its thresholds). Fail-open unchanged and now pinned end-to-end through the pool path: an observer registered from an uninstantiable module lets every tool call through (`a_broken_observer_fails_open_and_the_tool_still_runs`). The lifecycle shortcut claim was corrected while there: the watcher feeds only the kernel harness seam, so a newly-installed plugin needs a RESTART to appear as a live tool/skill/observer registration (audit S3) — the docs previously said "watcher re-scan covers it", which was half-true.

- **iters 1720-1722 — the WASM plugin architecture is awake (deepseek-harness import).** The Extism plugin host, manifests, discovery and adapters were shipped code that no default build ever compiled (`plugins-wasm` was default-off), so the plugin architecture ran in nothing the operator launches. It is now on by default, the capability map is complete, and the two dsh semantics operant lacked are ported. **iter-1720**: a plugin declaring the `skill` capability contributes its `skills/` bundle as a `SkillManager` extra root (operator pool wins collisions; Holon runs skip it; `/reload-skills` re-adds the roots so a rescan does not silently drop plugin skills). **iter-1721**: `PluginManifest.requires` names other plugins a plugin needs — an unmet require skips the plugin at discovery, transitively, with mutual requires satisfied (deepseek-harness `inject` parity collapsed to a filter; the named shortcut is that a returning provider does not fiber-reload — restart/watcher re-scan covers it). `manifest.toml` is the one manifest authority: the git installer validates it with the host's own parser, so install-time rejections name the same defect discovery would. **iter-1722**: the tool registry gained the veto-capable interception seam (dsh `tools/pre-execute`/`tools/post-execute` waterfall parity, verdicts reusing the vestigial `PipelineAction`) — a pre-block stops a tool before it runs and a post-block replaces the result, both as typed error results naming the reason; the empty-registry default stays allocation-free. WASM observer plugins ride it through a new `observe` export (`{"action":"allow"}` or `{"action":"block","reason":"..."}`), failing open so a broken observer logs instead of bricking the tool loop. The port deliberately did NOT carry Cordis's service-store/fiber machine, its five dispatch modes, or its patch language — plan: `docs/plans/2026-11-05-import-deepseek-harness-plugin-mechanism-architecture-into-o.md`.

- **iter-1640 — the presence registry finally has callers, so "live" means something.** iter-1634 shipped 263 lines of cross-process presence registry (`register_presence` / `unregister_presence` / `prune_dead_presence` / `session_is_live`) with **zero production call sites** — mechanism, tests and all, called by nothing. So `session_is_live` answered false for every session, and operant still could not tell "closed at Ctrl+C" from "crashed mid-turn". `claim_session_here` is now the single entry point a live process calls: prune other processes' dead records (so stale claims from SIGKILLs neither accumulate nor read live), claim for this pid (last writer wins — refusing would wedge every relaunch behind a zombie), and leave the previous owner gone by definition, which is what makes the interrupted status honest. The TUI claims at both agent-construction sites, and both exit paths release. Three new tests pin the wiring contract specifically, because the old tests all passed while nothing was wired.
- **iters 1640 + 1650 — the interrupted-session signal, with liveness.** The TUI teardown writes a one-generation exit marker naming the live session; a deliberate `/exit` deletes it first, so the marker means "cut off" and never "you chose to stop". `consume_marker` reads and deletes together, so the offer survives acknowledgement exactly once — a notice that cannot be dismissed stops being a notice. `/session` now opens the hub with the cursor already on that row (`open_selecting`), which is the keyboard path to the target the notice names; the other four hub-open sites are filter-specific or mid-session, where the marker's session is stale by definition, so they are deliberately not routed through it. **Not an auto-attach on either surface**: Ctrl+C is how an operator escapes a turn that is going badly. The row ordering is untouched — the interrupted-above-idle rank is another agent's Stage C, and `open_selecting_does_not_reorder_the_rows` pins that boundary.
- **iter-1642 — `sessions resume --last` and `sessions list --interrupted`.** iter-1's fifth item was never touched, so the operator had no CLI token for "what was I doing when I got interrupted". `--last` reads the exit marker instead of guessing from recency, so the operator sees *why* that session; with no marker it degrades to the same most-recent row rather than failing, which keeps the flag safe to alias. The filter reads the status field `list_sessions_rich` already carries — no second loader and no second ordering rule to keep in step.
- **iter-1646 — `/sessions` becomes a browser, so a telegram user can choose without an opaque id.** The gateway handler reported only *bindings* — session id, platform user, channel — so a telegram user had to copy an id out of a message they cannot read, run `operant sessions list` on the server, and paste it back. It now joins the binding table to the rich session loader and renders `1. *title* — 3h ago · 12 msg`, with `/sessions <n>` pinning that row through the same honesty gate `/resume` uses. The reply is capped and says so when it is, because a chat surface has no scrollback; and Slack is not claimed — it is outbound-only, so no Slack user can type `/sessions` at all.
- **iter-1640 — the gateway's interrupted-session notice.** iter-1634 gave the TUI a resume notice and the gateway nothing, so a telegram/discord operator had no path from "I was interrupted" to "resume that" — the equivalent was still `/new` + `/resume` by opaque id. A daemon has no status bar, so the surface is a console line on boot, which is where the operator who runs it actually looks.

- **iters 1631, 1635 and 1645 — a turn's four exclusive spans are captured and classified (RT-209, row 3 of the codex-architecture-deepen port). **Consumed at the seam since iter-1661, still no SUBSCRIBER reads the carried value** — the value is carried on `TurnEnd` and every subscriber reads only `session_id`, so the capture is live and the consumer is deferred.** `crates/operant-core/src/turn_attribution.rs` shipped as a dead module: exported at `lib.rs`, called by nothing, tests included, with a doc header claiming it was the RT-199 fix (it was not; RT-199 closed at iter-1505 through `agent/mod.rs`'s `tool_durations_by_call` map). The plan specified the volatile `<turn_context>` tail beside `<context_budget>`, and iter-1626 MEASURED why that seat cannot work: `build_messages` runs once before the loop starts, so at that moment no model call has happened and no tool has run, and a block placed there renders `n/a` for every span on every turn — the RT-199 failure wearing a new hat. Emitting on the last iteration does not work either, because the frozen prefix and history must stay byte-stable across the turn's requests or the provider cache is invalidated on every tool iteration. What shipped instead is the recommended option: **emit at turn END, off the request path.** `TurnEnd::attribution` carries the turn's classification; `OperantAgent::turn_attribution()` builds it from the turn's wall clock, every model call's folded duration, the per-turn tool total, and an honest `Unmeasured` for `user_wait` (the approval gate is a synchronous decision and the guardian LLM's time already sits inside `inference`). `AgentConfig::show_turn_attribution` is the kill switch and is reachable from `operant.toml` as `[agent] show_turn_attribution` (default on) -- iter-1645 threaded it through `BehaviorSettings` after iter-1631 shipped it as a hardcoded `true`, which made the switch unreachable. `None` means "no attribution attempted", which is a different fact from a span left `Unmeasured`, and subscribers must render it as unavailable rather than as zeros. iter-1631 wired three of the module's four public types; a post-ship audit counting external references per type found `TurnAttribution` still at zero callers, so **iter-1635 carries the classification** (`Disabled` / `Incomplete`-naming-the-gaps / `Complete`) rather than a bare breakdown, because a raw breakdown tells a subscriber that `tools` is unmeasured but not WHICH spans are missing, so it cannot tell "the model used no tools" from "nobody timed the tools". Six regression tests plus a strengthened emission-site guard; six mutations proved, each observed to fail BY NAME before restoring (including one that passed every runtime test and was caught only by the source guard, which is how the unpinned `run()` -> `TurnEndBus::emit` wiring was found).

- **iter-1500 — `/subagents`, the gateway surface for background delegations that did not exist.** `/agents` on a chat platform listed the gateway's *platform sessions* (a telegram chat, a discord channel) and nothing read the background-delegation registry at all, so an operator watching a running subagent had no view of it by any means. `/subagents` (alias `/subagent`) now lists that registry: one row per record with its dispatch id, the goal, the status (`running` / `completed` / `failed`), the model, how long ago it was dispatched, and — for a finished row — the child's answer or its error. The two commands answer different questions and never overlap, so the new reply cross-links `/agents` and, when the process has dispatched nothing, says WHOSE subagents are missing rather than claiming none exist anywhere: the registry is process-local, so an empty list is a fact about this process, not about the machine. Five regression tests, each pinned by a mutation that drops the goal from the row, mislabels a Pending record as completed, or stops reading the registry.

### Fixed

- **iter-1872 — the checkpoint store's exclude file was written with the error dropped (RT-297).** The file is the only thing that stops the shadow store from snapshotting itself — the code says so right above it — but both writes used `let _ =`, so a failed write left `ensure_store` returning `Ok` and `take_checkpoint` went on to `git add -A` into precisely that self-snapshot loop (the working-tree size bounds do not stop it: they measure the tree, not the store's own packed objects). **Fixed**: both errors propagate, and the caller already turns an `Err` from `ensure_store` into "checkpoint refused". **Pinned** by `ensure_store_fails_when_the_exclude_file_cannot_be_written` — mutation-proven. **Alongside**: the existing live-store repair test now uses a temp store with a stale exclude file, so it no longer depends on the operator's home being writable.

- **iter-1873 — the `cron` tool rejected schema-compliant calls (RT-298).** `scheduleDisplay` is `Option<String>` (so schemars does not list it as required) but the body demanded it, and no prompt or doc told the model the field exists — so a model that followed the schema could not create a cron job. And the tool computed `next_run_at` with a private copy of the schedule rule that did not accept the 5-field form its own doc advertises, so it answered `"next_run_at": null` for a job the database had already scheduled. **Fixed**: the display is derived from the schedule when the caller did not supply one (exactly what the CLI does), and the next run comes from the shared rule (`cronjobs::schedule::next_run_from_schedule`) — the one the database and the scheduler use; the private copy is deleted. **Pinned** by `cron_create_accepts_a_five_field_schedule_and_reports_the_next_run` — mutation-proven.
- **iter-1872 — the checkpoint store's exclude file was written with the error dropped (RT-297).** The file is the only thing that stops the shadow store from snapshotting itself — the code says so right above it — but both writes used `let _ =`, so a failed write left `ensure_store` returning `Ok` and `take_checkpoint` went on to `git add -A` into precisely that self-snapshot loop (the working-tree size bounds do not stop it: they measure the tree, not the store's own packed objects). **Fixed**: both errors propagate, and the caller already turns an `Err` from `ensure_store` into "checkpoint refused". **Pinned** by `ensure_store_fails_when_the_exclude_file_cannot_be_written` — mutation-proven. **Alongside**: the existing live-store repair test now uses a temp store with a stale exclude file, so it no longer depends on the operator's home being writable.

- **iter-1886 — the `tool_backend` tool advertised a switch it never performed and four search engines that no longer exist (RT-304).** The description said it "optionally switches the active backend" and the schema exposed a `backend` parameter, but the body never read it (there is no writer anywhere in the crate; the struct's `#[expect(dead_code)]` silenced the compiler) — and the table still advertised tavily, exa, searxng and ddg as web_search backends although plan 11C1 deleted all four. So a model could read the schema, ask to "switch web_search to tavily", and be told the switch happened against a list of engines that do not exist. **Fixed**: the tool is honest — the `backend` parameter is gone from the schema, the description says it reports (read-only) and does not switch, and the web_search backends are the one engine that is actually there. **Pinned** by `tool_backend_is_query_only_and_lists_no_deleted_provider` — mutation-proven. **Still a product decision**: a real backend switch is a new feature that needs a live selection seam.

- **iter-1883 — five places cut free text with a byte slice (RT-301).** A byte slice panics when the cut lands inside a multi-byte character — the repo already paid for this once ("the same defect that killed `operant sessions list` on the operator's own em-dash") — and the core still had five such cuts: a model id in the insights report, a memory file's first heading in the learning graph, a webhook response body, a discovered CLI's description, and an error body in the error path, where the panic would replace the error with a crash. **Fixed**: all five cuts are char-safe (the cut is taken over `chars()` and the same ellipsis is appended). **Pinned** by `no_free_text_is_cut_with_a_byte_slice` — a source-grep contract over the five files — mutation-proven.

- **iter-1875 — Ctrl+Enter always QUEUES, and a message queued while idle is now DELIVERED.** The operator's contract was "shortcuts like ctrl+enter must queue the message" and then "CTRL+ENTER still does not queue correctly": the key only queued while a turn was already streaming — while idle it was a second Submit, so the message went straight into a turn the operator never asked for. The key now means QUEUE in both states. The second half of the report is the delivery: the agent's steer drain runs BETWEEN iterations, so a message queued while the TUI was idle had no iteration boundary to wait for — it sat until the turn AFTER the next one (or forever, if that turn ended in a single iteration). The submit path now takes the pending queue and delivers it WITH the next turn, ahead of the draft the operator just sent (they queued first), and the queued band and the prompt history follow. **Pinned** by `ctrl_enter_queues_when_idle`, `an_idle_queued_message_is_taken_for_the_next_turn` and `queued_messages_keep_their_order` — mutation-proven (the idle-submit behaviour back → two failures by name).

- **iter-1872 — the context cap compacted at 35% of the window, so the agent's own history was truncated away on every turn.** The compaction caps defaulted to an ABSOLUTE 350k/400k, and the resolver takes the SMALLER of the absolute and the percentage axis — so a 1M-token window compacted at 350_000 tokens, 35% of itself, while 640k tokens of room sat unused. The preflight ladder then ran on EVERY turn of a long session, and its first rung re-trims every historical tool result to 2_000 chars — a LOSSY, content-agnostic truncation of the agent's earlier reads (the "generic compression"), applied turn after turn. That is why the agent stopped following instructions it had already read: the text it was working from was truncated out from under it. **Fixed**: the absolute axis is OFF by default (`context_soft_cap_tokens` / `context_hard_cap_tokens` = 0), so the percentage axis governs and the threshold tracks the window the model actually has (1M window → compact at 800k, never exceed 950k; a 128k window is unchanged at 102k/121k). The absolute axis stays available as an OPERATOR ceiling — set it and the smaller axis binds again. **Pinned** by `context_caps_default_to_the_window_percentage` and `a_one_million_window_does_not_compact_at_thirty_five_percent` (the measured case: 361_332 tokens against a 1M window) — mutation-proven.

- **iter-1871 — the clipboard notice outlived its TTL and never left.** The writer was not the defect: `set_status_notice` stamps a three-second TTL, but the notification line read `status_message` raw and the row-reservation check asked only whether the field was set — and nothing ever cleared it. So "Copied to clipboard via wl-copy." stayed on screen for the rest of the session and kept its row. **Fixed**: ONE TTL-aware reader (`App::live_status_notice`) serves every notice surface, the frame loop clears the dead field, and the write side has exactly two doors — `set_status_notice` (3s, for events) and the new `set_status_message_persistent` (for the few messages that are STATE: the archive y/n prompt, the copy-mode hint, "Agent init failed"). **Pinned** by four cases in `tui::app::notice_ttl_tests` — mutation-proven.

- **iters 1839/1841 — the operator's QUEUED messages were invisible, the messages they SENT were unreachable, and `/goal` never stood.** Three defects from one report, all on the "what the operator sent" path. (1) The steer queue was a LIVE mechanism the agent drained at every iteration boundary and a DEAD surface for the operator: `TuiState::queued_messages()` returned `&[]` behind a `[port-decision]` stub, so the queued-message band never rendered anything — a Ctrl+Enter mid-stream (or `/steer`, `/queue`) emptied the composer, flashed a notice, and the message VANISHED from the screen. The App now MIRRORS the live queue (refreshed on every push, every Ctrl+X recall and every agent-event drain) and the band renders the real queue. (2) `take_input` was the only writer of the prompt history, so a queued message and a `/goal` — messages the operator had actually sent — were unreachable from the Up/Down ladder and from Ctrl+R. `App::remember_prompt_history` is now the ONE writer, and the queue and `/goal` paths use it. (3) `/goal` submitted the goal ONCE as a user message while its own field doc promised "a system preamble message whenever it is set" — nothing read the field, so a goal set three turns ago had already fallen out of the conversation. The agent now carries a STANDING goal (`OperantAgent::set_standing_goal`), injected as `<session_goal>` into the volatile `<turn_context>` tail of EVERY turn until it is cleared, and cleared with the session on `/clear` and `/new`; it rides the tail rather than the frozen prefix, so a session with no goal produces a byte-identical request. `/goal off|clear|none|reset` drops it. **Pinned** by five TUI cases (`tui::app::steer_queue_tests`) and one core case (`the_standing_goal_rides_every_turn_until_cleared`) — both mutation-proven.

- **iter-1842 — the advisor card lumped every note into one sentence.** The preserve channel joined the notes with `"; "` into ONE string, so a three-note review read as a single run-on sentence, and a continue-channel review painted the model payload (the boundary guidance + the raw `<advisory>` XML) into the same card. The card now renders ONE BLOCK PER NOTE, severity-prefixed, blank line between blocks (`advisor::format_advisor_card`), and `advisor::advisor_review_summary` owns the delivery shape — header, card, then the continuation marker + batch AFTER the card — so the host splits the operator's card from the model's payload at the marker and the marker still rides the submitted query (the engine's continuation latch keeps working). **Pinned** by two core cases and two TUI cases (each note on its OWN rendered row; the marker and the advisory XML never painted in the transcript) — mutation-proven.

- **iter-1845 — the composer painted a tooltip inside the input area.** The queue shortcut ("Ctrl/Cmd+Enter to queue"), the shell-mode notice and the next-session notice were pushed into the SAME lines vector as the wrapped draft and painted at the composer's TEXT rect, so the first row of the box the operator types in was a line nobody wrote and the draft was pushed down a row (the caret, the copy snapshot and the placeholder all carried that offset). The hints are STATUS, not input: `composer_hint()` computes them and `draw_status()` paints them on the status row ABOVE the composer. The text rect carries nothing but the operator's own text, and the band no longer reserves a hint row (`input_hint_line_height` is 0 — a reservation with no painter silently swallows a transcript row, the defect RT-144 removed for the vim badge). **Pinned** by `the_input_area_carries_no_non_input_text` — mutation-proven.

- **iter-1871 — the FTS index was never backfilled (RT-296).** `messages_fts` is an external-content FTS5 table, so it starts empty and only the triggers populate it — they fire for new inserts only. The creation path created the table, created the triggers and returned, so the one moment a backfill could have happened was the moment it was missed, and there is no `rebuild` anywhere in the crate. On any database that held messages before the FTS table existed, those messages could never be found again: the search path the `session_search` tool uses joins `messages_fts` with no LIKE fallback (the LIKE path fires only on an error, and an empty-but-healthy index returns `Ok(vec![])`), so an install's whole pre-FTS history was silently unsearchable. **Fixed**: when the index is first created it is rebuilt once, inside the same migration pass, so a failure aborts the init rather than leaving a half-indexed database. **Pinned** by `fts_index_backfills_messages_that_predate_it` — mutation-proven.

- **iter-1870 — a broken recall engine read as "zero hits" (RT-295).** The adaptive retrieval's round gathered its candidates with `engine.recall(...).await.unwrap_or_default()`, so a store error became an empty hit list and the round came back `Ok` with no evidence and no leads — exactly like a session with nothing to recall; the model was told "no hits" and the real failure was never reported. **Fixed**: the recall's error propagates (`next_round` already returns a `Result`), so the failure surfaces to the caller instead of being flattened into "nothing found". **Pinned** by `a_failed_recall_is_not_reported_as_zero_hits` — the FTS table the recall reads is dropped so the recall really fails — mutation-proven.

- **iter-1868 — a failed insights query read as "no activity" (RT-294).** When the session query failed, the engine logged a warning and returned the plain empty report, so `session_insights` answered `{"empty": true}` and the model could not tell a broken database from a quiet week — the only trace was a log line. **Fixed**: the report carries the failure, the failed path fills it in, and the tool's output includes it. **Pinned** by `a_failed_insights_query_is_not_reported_as_an_empty_report` (the sessions table is dropped so the query really fails) — mutation-proven.

- **iter-1867 — the clippy gate caught two things iter-1861 and iter-1864 introduced.** `delegate_task`'s `call` and `dispatch_background` grew a seventh argument (the requested toolsets) and tripped `clippy::too_many_arguments`, and the new budget test's `let mut config = AppConfig::default(); config.database_path = ...` tripped `clippy::field_reassign_with_default`. The scoped gate is RED, which is not shippable, so both are fixed: the two functions carry the same lint expectation the file already uses for its builder, and the test builds its config in one expression. No behaviour change.

- **iter-1865 — the verification evidence ledger's write error was dropped (RT-292).** `verify_task`'s module doc promises it "records every phase in the `verification_events` evidence ledger", but the write's error was swallowed with `let _ =`, so a failed write left the ledger missing a phase while the tool returned success — and `operant verify evidence` showed a history that quietly disagreed with what the tool reported. **Fixed**: the error is reported (the recipe, the phase and the error are named in the warning). **Pinned** by `the_verification_evidence_write_error_is_not_dropped` — a source-grep contract — mutation-proven.

- **iter-1864 — `delegate_task`'s `toolsets` parameter was accepted and silently ignored (RT-291).** The model-facing schema carries `toolsets` at the top level and per task, but the body never read either: the child's toolset came from the parent's policy alone, so a model that asked "delegate this with only the terminal toolset" got a child with the parent's full set and nothing said so. **Fixed**: the request is honoured, **narrow-only** — the child's set is intersected with the requested one, so a request can only lower what the parent allows and can never add a toolset the parent lacks; an empty request means no toolsets; the narrowing is applied before the orchestrator's own delegation grant, so an orchestrator keeps the ability to delegate; the parameter now reaches all three arms (foreground single, background dispatch, swarm — a task's own request wins over the top-level one). **Pinned** by `the_requested_toolsets_narrow_the_child_set` — mutation-proven.

- **iter-1863 — `verify_task`'s `phases` parameter was accepted and ignored (RT-290).** The schema promises "For \"run\": subset of phases to run, e.g. [\"build\", \"test\"]" and the parameter was in the args struct, but the body never read it: `handle_run` detected the recipe and ran every phase it had — bootstrap, build and test — whatever the model asked for, and an unknown phase name got the full recipe with no error. **Fixed**: the requested subset is applied to the recipe before the run, and an unknown phase name is an error rather than a silent "run everything". **Pinned** by `verify_task_runs_only_the_requested_phases` (a Makefile recipe whose build and test phases each leave a marker) and `verify_task_rejects_an_unknown_phase_name` — mutation-proven.

- **iter-1862 — `[context_injection]`'s three aspect quotas could never fire (RT-289).** Five per-aspect character quotas are configured; only `dm_quota` (the DM tap) and `feed_quota` (the feed capture) have producers. `global_quota`, `dept_quota` and `self_quota` describe content classes retired to Holon, so those classes always return an empty list and the quota is never compared to anything — while the example file presented all five as live budgets. Unlike every other dead knob in this repo, these three did not report at load. **Fixed**: the three keys say "ACCEPTED BUT NOT APPLIED" in their own doc comments, the example file marks them UNREAD, and the loader REPORTS a set value (one line naming the exact keys to delete). **Pinned** by `a_context_injection_quota_is_reported_as_unread` — exactly one warning naming the two set keys and not `self_quota` — mutation-proven.

- **iter-1861 — every per-seat budget override was a silent no-op (RT-288).** `operant budget set <seat> --cap N` printed "Budget override set for '<seat>'", and the row went into the app database (`~/.operant/database.db`) — while both enforcement paths (the gateway's turn gate and the cron scheduler's budget gate) read the org sibling (`~/.operant/operant_kanban.db`). The override was stored, listed (`operant budget list` reads the same wrong file, so the operator saw their row and believed it was live) and enforced by nothing; the seat's cap was the global one. This is the same class the context-injection tables already paid for: the org family's rows live in the sibling of the app database, and a store opened at the app database path is a store the enforcement never sees. **Fixed**: the core owns the path (`SeatBudgetDb::init_org_sibling`, the same rule `org::context_injection` follows) and both the command and the gateway mount use it; rows an earlier build wrote into the app database are copied across once (recorded in the sibling), so an operator's existing override is not orphaned. **Pinned** by two tests — the core's (the row crosses over, the sibling is the file the enforcement opens, the migration runs once) and the CLI's (the command's row is visible to enforcement AND the command does not create the app database at all — without that assertion the one-time migration masked the wrong path) — both mutation-proven.

- **iter-1860 — the scheduler's comments described three things that do not exist (RT-287).** A doc block on the `observer` field described "iter-684 (gap 6): the daily seat-pairing sessions' settings ... the tick never checks" — a field that does not exist; a comment on the tick's early return claimed the socialisation coordinator "must be checked on EVERY tick", plus a second comment described the daily seat-pairing check — and neither check is in the tick (the coordinator is not part of this tree); and the doc of `seat_memory_append` ("appended by the socialisation coordinator") sat on a function that is `#[cfg(test)]`, so the production READER of a seat's `MEMORY.md` finds no file unless a seat writes one itself: the seat's "continuity thread" is never written by operant. The feature gap itself is already named (RT-136); what was left was the code claiming the opposite. **Fixed**: all three comments now state the truth. **Pinned** by `the_scheduler_does_not_claim_a_coordinator_check_it_does_not_have` — a source-grep contract — mutation-proven.

- **iter-1859 — the non-streaming context-overflow retry never failed typed (RT-286).** `[agent] stream = false` is a documented knob, and on that path the context-overflow compression arm ended in a bare `.await?`, so a second overflow after a compression that did not shrink the history escaped as the raw provider error — even though the arm's own comment ("the SECOND overflow fails typed") and the compression callee's doc both promise the typed failure. The rule existed in the streaming arm and in the non-streaming rotation arm; the port landed in the wrong arm. It matters because the gateway's recovery keys only on the typed error ("compact the stored history and retry the turn once"), so on the non-streaming path it never fired and the operator got the provider's message instead of "Run /compact, then send the message again." **Fixed**: the arm applies the same two-strike rule as its streaming sibling. **Pinned** by `the_non_streaming_compression_arm_fails_typed` — a source-grep contract over the arm whose needle is the code shape — mutation-proven.

- **iter-1857 — a rewind fork's message count was double its transcript (RT-284).** `rewind_to_turn` seeded the fork's session row with `message_count = prefix.len()` and then persisted the prefix through `save_message`, which increments the counter on every append — so the fork's counter ended at exactly twice the number of rows in its transcript. Every other producer of a session row seeds `0` and lets `save_message` count; the rewind was the only one seeding a non-zero value. The column is what the session listing, the gateway API's `message_count`, the insights totals and the session hub's "N msg" display, so every rewound session reported double its real size. **Fixed**: the fork's row is seeded with `0`. **Pinned** by `rewind_fork_message_count_matches_its_transcript` — the test reads the column (not `list_sessions`, which recomputes the count with a JOIN and would agree whatever the column says) — mutation-proven (left 2, right 1).

- **iter-1855 — two transcript writes dropped their error (RT-282).** The agent's transcript is the session's memory, and two writes to it dropped the error with `let _ =`: the rewind's fork prefix (while the sibling session-row write in the same function DOES surface its failure, so a failed fork produced a session whose history silently lost messages — a context hole the agent then works from) and the interrupted-tail commit that appends the cancelled tool rows after an abort (invisible: a transcript that lost those rows was indistinguishable from a clean one). **Fixed**: the rewind propagates the write error exactly like the session row's above it; the tail-commit logs the failure with the session id and the row's role. **Pinned** by `the_rewind_does_not_drop_the_transcript_write_error` — a source-grep contract in the repo's own shape — mutation-proven.

- **iter-1850 — a refreshed OAuth token that could not be written back was reported as "refreshed successfully" (RT-279).** The credential pool writes a refreshed token back to `~/.operant/auth.json`, and the write's error was dropped with `let _ =` at both call sites while the line above logged success either way. `persist_to_auth_store` writes atomically and can fail (a read-only home, a full disk, an unparsable store): when it did, the in-memory credential was good but the on-disk one stayed the pre-refresh token — after a restart the provider rejects that expired token and the operator sees an auth error with no hint that the write failed. **Fixed**: the error is reported at both call sites (the provider and credential named, and the message says the on-disk credential stays stale); the line above now claims only what happened. **Pinned** by `the_credential_pool_does_not_swallow_the_auth_store_write_error` — a source-grep contract in the repo's own shape — mutation-proven.

- **iter-1849 — `[cli] max_help_lines` was accepted, documented and exemplified, and did nothing (RT-278).** The key's own doc promised "Cap on the number of lines served when the agent asks for a CLI's full `--help` on demand. Default 60", and `operant.example.toml` listed it without the "stored, not yet enforced" annotation its two siblings carry — but nothing reads it: the only line cap in the CLI surface is the parser's hardcoded `MAX_HELP_LINES = 500`, which caps the lines *parsed* to build the capability roster, a different thing. **Fixed**: the doc says "ACCEPTED BUT NOT APPLIED ... it has NO effect today, and the loader reports it when it is set"; the example file carries the same annotation as its siblings; and the loader REPORTS a set value naming the key to delete. **Pinned** by `a_cli_max_help_lines_setting_is_reported_as_unread` — mutation-proven.

- **iter-1847 — a kanban run was never closed (RT-276).** A worker claims a task (`claim_task` inserts the `task_runs` row, sets `status = 'running'`, points `tasks.current_run_id` at it) and the work finishes through the LIVE path (`operant kanban complete`, the kanban tool) — which marked the task done but left `current_run_id` set and the run row `running` forever. The board's own diagnostics classify that state as a **zombie run** and tell the operator to run SQL by hand, and the only code that closed a run properly (`Dispatcher::complete_run`/`fail_run`) had zero production callers. The dispatch path even printed "Use 'operant kanban complete {}' when done." — pointing at the command that did not close the run. **Fixed**: `complete_task` reads the task's open run, closes it (`status = 'completed'`, `ended_at`, the outcome), clears `current_run_id`, and the completion event carries the run it closed. **Pinned** by `completing_a_claimed_task_closes_its_run` — mutation-proven. The failure path (no `fail_task`, so a dead worker's run stays open) is named, not guessed at.

- **iter-1843 — `[observability]`'s six runtime knobs were accepted in silence (RT-274).** Only `backend` is read (the observer consults it). The other six promise an OTLP endpoint, a service name, authorization headers, and a runtime trace mode, path and entry cap — and nothing reads them: an operator who set `backend = "otel"` and an `otel_endpoint` expected their traces to reach a collector, and nothing exports them; the runtime-trace trio, the section that would tell an operator what the agent is doing, records nothing. **Fixed**: each of the six says "ACCEPTED BUT NOT APPLIED ... it has NO effect" in its own doc comment, and the loader REPORTS a set value (one line naming the exact keys to delete). `backend` is not named — the observer reads it. **Pinned** by `an_observability_knob_is_reported_as_unread` — mutation-proven.

- **iter-1839 — seventeen provider-profile knobs were accepted in silence while their own docs promised the opposite (RT-272).** A `[providers.models.<name>]` profile carries seventeen keys that NOTHING reads (`api_path`, `extra_headers`, `wire_api`, `requires_openai_auth`, the three `azure_*`, `temperature`, `merge_system_into_user`, `provider_extra`, `pricing`, `native_tools`, `think`, `chat_template_kwargs`, the three `ollama_*`) — the client reads only `base_url`, `api_key`, `api_keys`, `model`, `timeout_secs`, `max_tokens` and `name`. Each of the seventeen promised a behaviour in its own doc comment: extra headers "sent with every request", credentials "pulled from `OPENAI_API_KEY`", pricing "merged into the cost-tracking lookup", native tools "consulted only by the Groq factory". None of it happens, and `operant doctor` hard-ERRORed on an out-of-range `temperature` — a knob that cannot change any request, so the operator was told their config was broken over a setting that does nothing. **Fixed**: each key now says "ACCEPTED BUT NOT APPLIED ... it has NO effect"; `ModelProviderConfig::unread_wire_knobs()` lists them next to the fields they name; the loader REPORTS a set value (one line per profile, naming the exact keys to delete); the doctor's temperature check is a warning that names the knob as not applied. The keys stay in the schema — deleting a field would fail every existing `operant.toml` (`deny_unknown_fields`). **Pinned** by `a_provider_profile_wire_knob_is_reported_as_unread` — mutation-proven.

- **iter-1836 — `${OPERANT_SESSION_ID}` reached the model as a literal (RT-270).** `[skills] template_vars` promises that `${OPERANT_SESSION_ID}` is substituted in SKILL.md before injection, and the substitution function is correct — but the only production call (`build_skill_invocation_message_in`, the one function the TUI's `/skill`, the gateway's `/skill` and `/skills` all come through) hardcoded `None` for the session id, so the literal token was injected into the model's context and the skill's own instructions told the model to use a placeholder that does not exist. `${OPERANT_SKILL_DIR}` WAS substituted, which is why the gap sat unnoticed in the shared path. **Fixed**: the builder takes the session id it is invoked in, and the surfaces that know it pass it (the TUI's current session; the gateway's runtime command's agent session id). The gateway's sync command arm carries no session id — its context holds platform/channel and the session key also folds in the chat thread — so there the token stays literal, the substitution function's documented fallback, now named in the config's doc rather than left as a silent difference between surfaces. **Pinned** by `the_session_id_token_is_substituted_before_injection` — mutation-proven (the literal token must never reach the model).

- **iter-1833 — the curator's persisted summary was always the PREVIOUS run's (RT-269).** `run_review` wrote `state.json` and only THEN set `last_run_summary`, so the file on disk carried the previous run's summary — and none at all after the very first run. `operant status` reads exactly that file, while `operant curator status` read the live value: two panels, one run apart, and the operator's status line described a run that had already been superseded. **Fixed**: the summary is set before the state is written, so the persisted copy is the current run's. **Pinned** by `the_persisted_summary_is_the_current_runs` — two runs with different outcomes, then the file on disk must carry the SECOND run's summary. Mutation-proven: the old order (save, then set) fails it by name.

- **iter-1832 — the curator's "never archive a skill a cron job calls" rule protected NOTHING (RT-267).** Three defects stacked on the same rule: the curator opened `<skills>/.curator/cron.db` while every cron job writes `<database dir>/operant_cron.db` — so its protection set was always empty; the automatic path (the agent's curator daemon) had no cron guard at all, so it archived cron-referenced skills while the manual `operant curator run` skipped them; and a failed read of the reference set was logged and replaced with an EMPTY set, which reads as "nothing is protected" — one transient sqlite error became the permanent archival of a skill a cron job still calls. The curator also dropped the errors of the writes that make an archive durable, so `SKILL.md` could end up in `.archive` while `usage.json` still said Active. **Fixed**: the core owns the cron database path (`cronjobs::db_path`) — one rule, one file — and the CLI's `operant cron` and the curator now open the same one; both curator paths read the protection set through ONE helper that FAILS CLOSED (an unreadable set aborts the pass instead of archiving blind); the automatic path applies the same cron guard and counts the skips (`skipped_cron`); the tracker's and the state's writes are propagated instead of dropped, so an archive is durable or the pass reports why it is not. **Pinned** by `the_automatic_curator_never_archives_a_cron_referenced_skill` (mutation-proven: the guard back → archived 1, expected 0) and `an_unreadable_cron_reference_set_aborts_the_review` (mutation-proven: the empty-set fallback → the review no longer aborts).

- **iter-1831 — the operator's configured `[skills] root_dir` was ignored by seven callers (RT-265).** The skills TOOLS and the agent's skill manager honour the setting, and the kernel tool has a config-aware branch — but seven other callers reached for the platform default directly: the curator's auto-run, the skill-bundle loader, `skill marketplace install`, `skill marketplace updates`, the status panel and the setup dump. With a custom root that means the curator archived the lifecycle of a tree the operator does not use, every skill in a bundle came back "missing" (the bundle injected nothing), `skill marketplace install` wrote the skill **where the agent never looks** — the install "succeeded" and the model could not see the skill — and the status/dump panels named a directory nobody configured. Invisible until the setting is used, and then every skill-facing feature except the tools points somewhere else. **Fixed**: one rule — the skills tree is the operator's configured root. `crate::skills::configured_skills_dir()` resolves it from the runtime config (falling back to the platform default when the setting is empty, the same rule the kernel tool applies), the core callers use it, and the CLI callers use the config they already hold. The bundle loader's "config-aware" comment is now true. **Pinned** by `the_bundle_loader_reads_the_configured_skills_root` — mutation-proven (putting the platform default back fails it by name: "the bundle must resolve").

- **iter-1825 — the session-reset policy was a setting that did nothing (RT-263).** `[agent] session_reset` was parsed, offered by `operant setup` with four behavioural promises ("Reset when system prompt changes", "Reset every new session — lowest cost"), shown in the setup status panel as an ACTIVE setting — and read by NOTHING: no code path resets a conversation when the system prompt or the tool set changes, and the gateway's own `SessionResetPolicy` is always `ResetMode::None`. An operator who picked "Always" was told their sessions reset, and paid the API cost of a context that never reset. **Fixed**: the setting now tells the truth. A SET value is reported by `AppConfig::unused_setting_warnings` ("accepted but unread — it has no effect"), the setup wizard no longer offers it, the setup status no longer lists it as active, and the config template carries it commented out with the same note. The key itself stays in the schema (deleting it would break any `operant.toml` that sets it — the config carries `deny_unknown_fields`), so existing configs still parse. **Pinned** by `a_session_reset_policy_setting_is_reported_as_unread` — a config with `session_reset = "always"` must produce exactly one warning naming the key, while a default config stays silent. Implementing the policy itself (what a reset clears, and when) is a feature with its own semantics, and it is named in the ledger rather than guessed at here.

- **iter-1821 — the curator archived skills that were in daily use (RT-262).** The curator archives an agent-created skill after `archive_after_days` of INACTIVITY, measured from `last_used` in `.curator/usage.json` — but nothing ever recorded a USE: `UsageTelemetry::bump_use` / `bump_view` existed, were documented as "Called when a skill is actively used (e.g., loaded into the prompt path)" and "Called from skill_view or skill discovery", and had ZERO production callers. The only writer of that file was the skills tool's `record_usage`, whose action map covered create/delete/patch only. So `use_count` stayed 0 and `last_used` stayed at creation time for every skill, and a skill the agent invoked every day was archived 30 days after it was created like an abandoned one. **Fixed**: the invocation path — `agent::skill_preprocessing::build_skill_invocation_message_in`, the ONE function the TUI's `/skill`, the gateway's skill commands and the agent's own lane all come through — now records the use (through the curator's own `with_exclusive_lock` bridge, so the write cannot lose a concurrent record), and `skill_view` records a view, which moves the same clock. Telemetry never breaks an invocation: a failed write is swallowed. **Pinned** by `a_used_skill_is_not_archived_by_the_curator` — an agent-created skill whose last use was 40 days ago, invoked once through the production path, survives a curator pass with a ONE-day archive window. Mutation-proven: without the recording the curator archives it ("a skill in daily use must not be archived", archived 1, expected 0).

- **iter-1819 — a plugin's observer ran TWICE after its first live change, and a withdrawn observer kept watching (RT-260).** The registry holds two interceptor lists — an anonymous one (iter-1722's general seam) and a named, replace-by-plugin-name one (gap G6's live re-registration) — and `execute` consults BOTH. The boot path registered a plugin's observer into the anonymous list while the watcher registered it into the named one: so after the first watcher-detected change the same plugin's `observe` export ran twice per tool event, and the boot copy still held the pre-change module (the pool is resident, so the old module stayed alive and kept answering). The watcher also had no "else": a change that dropped the `Observer` capability — or dropped the `[config] turn_gate = true` opt-in — left the registration live, so a withdrawn observer kept seeing every tool call and a withdrawn turn gate kept vetoing turns; `unregister_plugin_interceptors` / `unregister_turn_interceptors` existed for exactly that and had no caller. **Fixed**: the boot path registers into the NAMED list (one plugin, one registration, replaceable and removable), and the watcher now withdraws what a change removed — the observer interceptor when the capability is gone, the turn interceptor when the turn gate is gone. The missing observability is added too: `ToolRegistry::tool_interceptor_count()` counts the anonymous list, because "how many interceptors will run for this tool?" had no answer while the two lists were consulted together. **Pinned** by three tests against the REAL fixture module: `a_boot_registration_and_a_watcher_change_leave_one_observer` (mutation-proved: the boot path back in the anonymous list fails it by name — left 2, right 1), `a_change_that_drops_the_observer_capability_stops_observing` and `a_change_that_drops_turn_gate_no_longer_vetoes_turns` (both mutation-proved). The anonymous seam itself stays as iter-1722's general extension point; after this fix it has no production user, and that is named in the ledger rather than silently deleted.

- **iter-1816 — the deferred bridge hides a tool by NAME while the tools array hides it by TYPE (RT-259).** `assemble_tools` partitions on the registry-stamped `ToolExposure` (the type's own decision), but the three bridge arms that REACH a deferred tool still asked its name: `tool_search`'s catalog filter, `tool_describe`'s guard and `tool_call`'s guard all ran `is_deferrable(name)` (an `mcp_` prefix plus a sourcehound-composite table scan) — and so did the turn-driven `materialise_for_turn`. The two rules agreed only because both production families happen to satisfy both. The tool module's own test proved the disagreement was real (a type-deferred tool with no `mcp_` prefix lands in the deferred set), and nothing exercised the bridge for that fixture — so such a tool was hidden from the model AND refused by all three arms: permanently unreachable. **Fixed**: every arm reads the stamped exposure (`s.exposure.is_deferred()`), so a third family can defer by declaring itself so without borrowing a naming convention; `tool_call` now checks availability first, so an absent tool is reported as absent rather than as "not deferred". `is_deferrable` (and its composite-table helper) are deleted — the name rule had no production purpose left, and a second rule is exactly the class of drift this repo keeps repairing. The sourcehound composite guard is upgraded with it: it asserted each composite's NAME was deferrable, so it kept passing if the type stopped deferring (the ~4,738-token RT-194 regression would have shipped silently); it now asserts the TYPE. **Pinned** by `a_type_deferred_tool_is_reachable_through_the_bridge` (all three arms, plus the executor), mutation-proven (restoring the name filter fails it by name: "the type-deferred tool must be in the catalog: []"). The fixtures that modelled MCP tools by name only (`DeferredDummy::new("mcp_…")`) are deferred by type now, as the registry types them.

- **iter-1813 — the operator's `[proxy]` table reaches the model client at last (RT-258, RT-131's residual).** The `[proxy]` table was in the config schema the whole time and the CLI applied it at startup (`config::apply_startup_proxy`), but **no client ever read it**: `apply_runtime_proxy_to_builder` had zero callers outside `operant-config/src/schema/proxy.rs`, so a deployment behind a proxy had no way through while the template promised "Outbound HTTP proxy for provider and tool clients". RT-131 closed the AMBIENT half in iter-1116 (a stale `HTTP_PROXY` from another tool rerouted every model request) and named this half as its residual: "plumb `[proxy]` into `ClientConfig`". The provider clients — the OpenAI-compatible one (both its request and its streaming client) and the Anthropic one — now apply the operator's explicit proxy on top of the no-proxy builder, so the environment still can never redirect provider traffic (RT-131 stays true) but a configured proxy does. `scope = "environment"` deliberately does NOT reach the provider: that is the ambient path RT-131 closed; use the default `scope = "zeroclaw"` or `scope = "services"`. The service key follows the endpoint (`provider.openai`, `provider.openrouter`, otherwise `provider.compatible`). Pinned by `configured_proxy_is_used_by_the_provider_client` — a capture listener standing in for the configured proxy must serve the provider request — mutation-proven (dropping the apply call fails it by name: "the configured proxy must carry the provider request", left 0, right 1); RT-131's own guard (`env_proxy_is_ignored_by_the_provider_client`) still passes. The template now says which clients the table reaches.

- **iter-1798 — the config template advertised a plugin MANIFEST's fields as CONFIG keys, and the loader rejects both (RT-246).** `operant.example.toml`'s `[plugins]` block documented a plugin's `manifest.toml` with live TOML assignments (`requires = ["other-plugin"]`, `provides = ["memory.backend"]`). Those are the manifest's fields, not `operant.toml` keys, and every settings struct uses `deny_unknown_fields` — so uncommenting the example produced "unknown field `requires`" and the WHOLE config stopped loading. The template's own gate was RED at HEAD and named both lines; it is green again now that the template points at `docs/library/plugins.md`, the one place the manifest schema is documented.

- **iter-1798 — `[skills]` paths resolve `~` and `$HOME` (RT-197).** The skills paths were the only path settings with no expansion, so a value copied from the template reached the filesystem verbatim and a skills operation created a directory literally named `$HOME` under the working directory. `root_dir` and `memory_dir` now resolve a leading `~` and a leading `$HOME` (and the `HERMES_SKILLS_DIR` env override gets the same treatment); an absolute path is untouched. Pinned by three unit tests, each mutation-proved.

- **iter-1808 — the workspace lint gate is green again: the expectations became allows (RT-255).** `rust-toolchain.toml` pins `channel = "stable"`, so the lint set moves with the toolchain. The 13 `#[expect(...)]` attributes iter-1804 added to answer one stable's diagnostics became `unfulfilled_lint_expectations` warnings on the next — and the gate treats a new warning as a failure. They are now `#[allow]` with the same reasons: the lints have no true positive in this workspace, and an expectation on a floating toolchain is a coin flip. `bash scripts/clippy-warning-gate.sh` → "seen=0 allowlist=0 new=0 stale=0 — pass".

- **iter-1803 — an observer's post-execute envelope carries the call's arguments again (RT-250).** The documented envelope is (point, tool, args, result, config), and the post-execute event shipped `args: null` — so the redaction/enrichment seat an observer provides (the `replace` verdict, gap G2) could see a tool's output but not the input it had to judge. The args now ride the envelope at every point, and a Replace keyed on them lands in the result. Pinned by a mutation-proved test.

- **iter-1800 — a delegation that produced nothing is a named failure, not an empty result (RT-249).** The parent's tool result IS the child's final message, and a child turn can end with no answer (the wall clock, the iteration budget or the grace gate can each stop it). The delegation then reported a successful call whose content was the empty string — a result the parent model cannot act on and cannot tell from a child that had nothing to say. A child that produced no text is now a named failure ("The sub-agent finished its turn without producing any output") across the sync, swarm-worker and background paths.

- **iter-1799 — the self-extension tools could never run: `harness_mount` / `harness_unmount` refused every call (RT-248).** Both gate on `ToolContext::metadata["approval"]`, and nothing ever set it — they are not permission-gated (so they are never prompted for) and the agent's tool context carried only the session id. The operator's `command_allowlist` DID reach the agent, but the approval stopped one hop short of the tool, so the refusal told the operator to approve a tool they had already approved. The agent now passes the approval on for a tool its allowlist covers.

- **iter-1799 — a failing aft command reports the child's own diagnostics instead of "unknown error" (RT-247).** The bridge already drains the child's stderr into a ring, but the failure path never read it: a failing `aft_inspect` answered the model with "unknown error" — no cause, no way to act. A failure with no usable message now carries the child's last stderr lines, and a literal "unknown error" keeps its name and gains the diagnostics in parentheses.

- **iter-1789 — `operant sessions list` died on the operator's own session title.** The table truncated a title with a BYTE slice (`&title[..25]`), which panics when the cut lands inside a multi-byte character — and the operator's title carries an em-dash: the command printed a panic ("end byte index 25 is not a char boundary") instead of the session list, in the one command an operator runs to SEE their sessions. The same byte-slicing shape was in four other display paths (the checkpoint reason, the masked API key, the TUI's session id and the stats dialog's model id) and is now char-safe everywhere in the CLI's display code. Pinned by `cmd_sessions::tests::a_title_with_a_multi_byte_character_is_truncated_without_panicking`.

- **iter-1787 — the self-improvement path stopped storing facts.** The session distillation asks the model for a JSON array of durable facts and parsed the reply with `serde_json::from_str`, so a reply that wrapped the array in prose ("Here are the facts: [...]") failed on its first character — MEASURED on the operator's live gateway log: 27 distillations failed that way, each a session whose facts were dropped instead of retained. The payload is the ARRAY: the parser now takes the first '[' that opens it and the last ']', so prose, a trailing sentence, or a fence no longer lose the facts. Two new unit tests pin both shapes.

- **iter-1787 — a resumed session keeps its ENTIRE tool history.** A tool row persisted WITHOUT its `tool_call_id` cannot be paired with the assistant turn that requested it, and the rehydration's orphan repair DROPS it — and then strips the calls of the assistant turns that asked for them, so the turn loses both halves. The writer has carried the id since iter-1769, but every row written before that has none and nothing repaired them: MEASURED on the operator's live database, 407 of the 437 tool rows in one session had no id (and ~25 000 rows across the other live sessions), and that turn's own repair log counted **818 dropped halves** — the whole tool history, gone on every resumed turn, which is exactly what "the resumed sessions do not have their entire context" describes. The rehydration now recovers the missing ids from the transcript's ORDER (an assistant turn's tool results follow it, one per call, in the order the calls were made); a row that already carries an id keeps it and claims its call, so a turn written partly before the fix and partly after it pairs correctly, and the tool NAME is recovered the same way. Measured on the live data with the shipped algorithm: ~25 000 tool rows recovered across six sessions, with **0** rows paired against a call no assistant turn made. Pinned by `crates/operant-core/tests/session_tool_pairing.rs`.

- **iter-1783 — the context calculation tells the truth, the resumed-session log stops lying, and an early turn no longer dies with an empty answer.** Three defects, one live session's worth of evidence (`~/.operant/logs/gateway.log` + the live database, 2026-11-05/06). **(1) The token estimate charged context the provider never charges.** `estimate_message_tokens` counted the assistant's reasoning, and the reasoning is not context: the Anthropic adapter drops it (`build_assistant_content` emits only text and tool_use blocks) and the OpenAI adapter sends it as `reasoning_content`, which the provider does not bill against the window — MEASURED against the live `large-stack` endpoint: 18 000 characters of `reasoning_content` added **15** prompt tokens to the next request (4 027 total, 3 904 of them cached). The live session carried 428 693 bytes of reasoning, i.e. ~107 000 phantom tokens on every request, which is why the agent compacted a request the provider was serving in ~200 000 tokens and why every turn printed "Compacted context: 418711 → 341122 tokens". The estimator no longer charges the reasoning; the soft cap (350 k) and the hard cap (400 k) now mean what they say. **(2) The session-rehydration warning always fired and always lied.** `rows_dropped` was computed as `total - start`, and `start` is already `total - kept`, so the two cancelled and the log reported the KEPT count as the dropped one: "rows_total: 643, rows_kept: 643, rows_dropped: 643" — every load claimed a mid-session start for sessions that lost nothing. It now reports the rows actually dropped, and warns only when rows were dropped. **(3) The auto-poke: a bounded synthetic continuation for a turn that ended with nothing.** A turn that runs out of room ends with a TOOLLESS wrap-up; when that wrap-up came back empty or degenerate (the live incident: a 13-character reasoning-leak artifact the S1 quality gate threw away) the turn stopped with NOTHING — an empty answer and a notice blaming "the wrap-up ran out of room mid-summary", which is not what happened. The core now issues a bounded poke — jcode's mechanism (`crates/jcode-todo-policy/src/lib.rs` `build_auto_poke_message`, `commands_auto_poke_errors.rs` `is_non_retryable_auto_poke_error`) — that gives the turn one more pass WITH its tools, bypassing the iteration budget and the wall clock that just stopped it. Bounded three ways: at most `MAX_AUTO_POKES = 2` pokes per turn, each granting `AUTO_POKE_ITERATIONS = 3` iterations past the limit; only when there is work to continue (an open todo list, or a tool call that SUCCEEDED — a turn whose every tool failed is a stuck model, and poking it would only repeat the failing calls); and never after a non-retryable provider error. A poke is persisted as a user row (the transcript records what the model was told) and the renderer recognises it, so a restored session shows "Auto-poke" instead of a message the operator never sent. The "Turn ended early" notice now names the cause that actually happened: a summary cut off by the output limit is "out of room"; a summary the quality gate rejected is "no usable summary". Pinned by `crates/operant-core/tests/auto_poke.rs` (the poke fires, bypasses the limit, is bounded, and does not fire when the wrap-up produced an answer), `tests/session_rehydration_log.rs` (the dropped count, and the absence of a false warning), and the estimator's own unit test.

- **iter-1777 — a tool that ran and failed now says so in the log.** The refusal paths (tool not found, bad arguments, blocked by approval, denied by seat policy) each log themselves; a tool that EXECUTED and returned an error did not. The failure reached the model and the operator, and nothing else — so the log of a turn whose tool failed could not say which tool failed, how long it took, or why. The loop now writes one line with the tool, the call id, the duration and the error text. Pinned by `crates/operant-core/tests/tool_failure_logging.rs`, which captures the tracing output and asserts the line names the tool, the duration and the error.

- **iter-1776 — the clippy gate is green again.** The gate compares the workspace's warnings against a committed allowlist, and the allowlist was EMPTY — so every warning in the tree was "new" and the gate was red at tip with four findings on lines nobody had touched: two `assert_eq!` over booleans in the patch tool's occurrence rules (`clippy::bool_assert_comparison`), a plugin fixture test that used `.expect()` without the test-target exemption header every other test target carries, and a `#[expect(clippy::expect_used)]` attribute in `agent/run.rs` that sat on `turn_attribution` (which has no `.expect()` at all) instead of on `run` (which has four) — so the one function that needed the exemption was uncovered and the one that had it was marked for a lint it never triggered. The attribute moved to the function it describes, the two boolean assertions became `assert!(!…)` / `assert!(…)`, the fixture test carries the standard header, and the fixture's runtime assertion uses `std::hint::black_box` so clippy stops reading it as a constant. `bash scripts/clippy-warning-gate.sh` now reports `seen=0 allowlist=0 new=0 stale=0 — pass`.

- **iter-1774 — the restored transcript, the silent stream end, and the replayed tool call: the parts the operator could not see.**

  **A restored session showed none of the turn's activity.** Both rebuild paths (the session hub's switch and the outer drain's rebuild) carried the same lossy conversion inline: the assistant's reasoning was dropped and every tool row became a bare system line — so a session restored after a restart looked like the agent had never run a tool. One shared converter now builds the display the way the LIVE path does: the reasoning is a `Thinking` block, the tool call is a tool row anchored after the assistant turn that asked for it, and the matching result fills that row's output. An orphan result (its call is not in the transcript) keeps its content visible instead of vanishing.

  **A stream that ended with no signal at all was taken for a finished answer.** The terminal `[DONE]` marker was parsed and thrown away, so a provider that closed its SSE body without it — and without a finish reason — was indistinguishable from one that answered completely: the operator saw a sentence cut off mid-word with no explanation. The marker is now surfaced, the OpenAI adapter marks such a stream `incomplete`, and the agent asks for a continuation when the text itself looks cut (a reply that ends on punctuation is finished, so a provider that omits the marker is not charged a pointless continuation on every turn). The answer is reassembled from both halves.

  **A recovered turn ran its tools invisibly.** When a stream died mid-read the loop re-issued the request, and the replay re-emitted every `ToolStart` the dropped stream had announced — under the same call ids. The dedup set (which exists so one call is not announced twice WITHIN a stream) still held those ids, so the replay's announcements were suppressed: the recovered turn's tool rows never appeared, and a row that had been announced kept the truncated arguments of the dropped attempt. The set is now cleared before the replay.

  **The debug log had a blind spot.** The inline picker consumes its keys before the debug publication, so the picker's keys were the one surface the event log could not see — exactly the surface a "the picker ate my key" report needs. The publication moved above the picker's gate. A live agent error is now recorded in the debug hub (the transcript card shows the operator; the log is what a bug report can attach), and a TERMINAL error exit dumps the event log — the clean exits already did, so the one run whose log the operator wanted was the one that was lost.

  Regression tests: `crates/operant-cli/src/tui/app/session_display_tests.rs` (the restored transcript keeps its reasoning, its tool row and its result; an orphan result stays visible; a tool-only turn commits no empty message), `crates/operant-core/tests/stream_incomplete_continuation.rs` (a stream that stops mid-sentence is continued and reassembled; one that ends the sentence is not), `crates/operant-core/tests/stream_replay_tool_rows.rs` (the replay re-announces its call — mutation-proved: without the fix the count is 2, not 3), three adapter cases in `crates/operant-core/src/agent/clients/openai.rs` (the completion signal), and two TUI cases in `crates/operant-cli/src/tui/app/debug_log_tests.rs`.

- **iter-1769 — the context caps, the resume, and the early-turn recovery: the core keeps what it promised.** Five defects, one campaign.

  **The caps are real now.** `[agent]` gained four knobs — `context_soft_cap_tokens` (350 000), `context_soft_cap_percent` (0.80), `context_hard_cap_tokens` (400 000), `context_hard_cap_percent` (0.95) — and BOTH axes bind: the smaller one wins, so a 1M window compacts at 350k and never exceeds 400k, while a 128k window keeps the old 80%/95% behaviour. The soft cap is the turn-boundary compaction trigger (crossing it forces the compaction ladder to run — without that the 350k cap would have been dead under a 1M window); the hard cap is enforced BEFORE EVERY provider call, so a turn that grows past it mid-stream compacts the request list instead of sending an over-budget request, and a context that cannot be compacted under the cap fails typed (`ContextExhausted`) instead of hitting a provider 400. The compaction rewrites the REQUEST list, never the durable transcript.

  **A resumed session keeps its whole context.** Tool rows were dropped on rehydration (`row_to_message` accepted only user/assistant), tool results were persisted without their `tool_call_id` (so a resumed tool request became an orphan and the request builder stripped the whole tool turn), and assistant messages were persisted through the short writer that drops reasoning and `finish_reason`. All three are fixed, and the core now writes the transcript exactly once per message — the gateway used to write the same user and assistant rows again, so every turn showed the operator's message and the bot's answer TWICE.

  **A truncated answer is no longer half an answer.** `finish_reason="length"` injected a bare "continue" prompt: the model could not see what it had already written (so it restarted) and the first half of the answer was dropped from the result. The cut-off text now rides the continuation prompt and is prepended to the final answer.

  **The wrap-up that ran out of room is continued, not lost.** The grace summary — the last call of a budget-exhausted turn — was neither added to the conversation nor persisted, and a streaming failure escaped through `?` with a raw transport error, past the turn diagnostics and the error event. The summary is now continued (bounded) when the provider cuts it off, kept in the conversation and the transcript, and a failed wrap-up reports through the same diagnostics every other turn failure uses.

  **The TUI's numbers stopped lying.** The context overlay showed the session's CUMULATIVE spend as if it were the live context occupancy (a 200k window read "full" after one long turn) — it now shows the provider-measured prompt size. The "session: Nk in, Nk out" line showed the CURRENT turn's output — it is now the session's cumulative input and output. `/clear`, `/new` and a session switch reset every per-session counter, and they wait for the agent task: `/stop` clears the streaming flag immediately while the cancelled task is still committing its tail, so a boundary command that landed in that window wiped the session the tail was writing into. The configured `context_window` is also the denominator everywhere — a custom alias showed 200k while the core ran with the configured 1M.

  Regression tests: `crates/operant-core/tests/context_caps.rs` (soft cap, its control, mid-turn hard cap), `crates/operant-core/tests/session_transcript.rs` (one row per message), the strengthened truncation case in `tests/loop_recovery_paths.rs`, two new grace cases in `tests/grace_output_gate.rs`, and four TUI cases in `crates/operant-cli/src/tui/app/tests.rs`. The cap tests are mutation-checked: disabling the mid-turn enforcement fails the hard-cap case, and disabling the soft-cap trigger fails the soft-cap case.

- **The input box showed the failed prompt twice — the operator's "leaking text in the input box" report.** The submit path pushes the user message into the transcript *before* the turn runs (so the prompt is visible the instant Enter lands, and the turn has a conversation to append to). When that turn then failed, `restore_failed_input_to_composer` handed the text back to the composer while the transcript row stayed, so the same sentence rendered twice — once as a transcript row and once in the editable composer, with nothing telling the operator which copy was which. The failed submission's user row is now removed with the restore (`App::drop_transcript_user_row_matching`, `crates/operant-cli/src/tui/app/messaging.rs`), matching codex's non-duplicating behaviour from the other side (`restore_user_message_to_composer` rebuilds the composer from the failed message without leaving a second copy, `chatwidget/input_restore.rs:543`). The removal is bounded three ways so it cannot eat real history: reverse search from the tail, exact content match, at most one row — so a transcript that legitimately repeats a prompt keeps its earlier copy, and a restore whose text never reached the transcript (a queued steer, a slash-command expansion) removes nothing. Turn-attribution metadata stays index-aligned through the removal (`sync_turn_metadata_to_messages`). Verified end to end: `operant tui debug simulate --keys 'hello world<enter>'` now renders one `hello world` row where it rendered two. Four regression tests, mutation-proved RED by deleting the call in `restore_failed_input_to_composer`.

- **Shift+Enter / Alt+Enter / Ctrl+Enter were untypeable, and therefore untestable — the operator's "SHIFT+ENTER is not working" report.** The root cause was in the debug harness, not the key handler (which has mapped Enter+SHIFT to `insert_newline` since iter-120 and has a passing unit test for it). `parse_generic_chord` in `crates/operant-cli/src/cmd_tui_debug.rs` required *exactly one character* after the `+`, so `<shift+enter>` fell through to `None` and `parse_key_sequence` then **typed the whole token as its literal characters** — eleven keystrokes of `shift+enter` into the composer. No scenario could exercise the newline chord, and the operator's report could be neither reproduced nor ruled out from the harness. `chord_key_code` now names the non-character keys a chord is realistically written with (Enter, Esc, Tab, the arrows, Home/End, Insert/Delete, Backspace, BackTab), and the single-printable-character path is preserved so unknown names keep the literal-fallback contract (`<ctrl+ab>` still types literally rather than silently truncating to `<ctrl+a>`). Verified end to end: `ab<shift+enter>cd` renders two composer lines and submits nothing; the same for Alt+Enter, Ctrl+Enter and Ctrl+J. Two regression tests, mutation-proved RED by removing the branch.

- **Two corpus scenarios pin the submit/newline surface — and one of them is a negative result.** `shift-enter-newline` (keys `ab<shift+enter>cd`) is mutation-proved RED: with `chord_key_code` deleted it fails on `not-contains:shift+enter`, which is exactly the eleven literal characters the old parser inserted. `failed-turn-no-duplicate-prompt` (keys `hello world<enter>` with `error-modal.json`) is mutation-checked and is NEGATIVE — it does NOT catch the duplicate it looks like it covers, and its notes now say so. The headless mock branch (`else if is_mock` in adapter_types/tui_app.rs) pushes the user message without routing through the live submit path, so with the `drop_transcript_user_row_matching` call deleted, rebuilt, and run, the scenario still renders one `hello world` and still passes. It pins the restore-once outcome cheaply and deterministically, but the duplicate is only reachable on the live path, which the mock cannot reproduce; the four unit tests in app/tests.rs cover that one and are mutation-proved RED. Recorded so nobody reads a green corpus as coverage for the fix. Both scenarios are double-render proven for determinism.

- **iter-1630 — the rewind reveal left a short zone scrolled entirely off-screen.** Found by re-deriving iter-1601's `reveal_offset` against oh-my-pi's own source instead of trusting its green tests. The function carried three match arms and the FIRST swallowed the second: `bottom <= committed → committed`. A picked turn sitting wholly above the viewport has a small `bottom`, so that arm answered first and left the pane unmoved while the outlined turn was entirely off-screen — the exact symptom iter-1601 existed to fix, still reachable for a short zone. Traced against `revealRange` (scroll-view.ts:310-341) and `scrollOffsetForRow` (scroll-viewport.ts:62-93) with `margin=0`: omp has no such arm and checks `top < visible.start` first, then `bottom > visible.end` (active row `bottom - 1`, offset `bottom - height`). The extra arm came from an unrelated `_top`/`bottom` warning cleanup, not a deliberate change. Verified with a standalone transcription of omp's rule across 10 cases (oversized, under-view, fully-inside, zero-height, `end == start`): the fixed arms agree on all 10 and the shipped ones disagreed on exactly one (`committed=20, start=3, end=6, h=10` → omp reveals at 3, shipped left the pane at 20). Regression test `the_reveal_offset_matches_omps_traced_arithmetic`, mutation-proved four ways: re-inserting the swallowing arm fails with `20 != 3`, deleting arm 1 fails, swapping the arms fails, and dropping `- body_h` from arm 2 fails.

- **iter-1625 — typing a filter in the rewind selector dragged the cursor to the OLDEST match.** The live-jump on every keystroke was `matches.first()`, so an operator who had walked the outline up several turns and typed a query lost their place on the first character and had to walk up again. oh-my-pi's `#filterChanged` (rewind-selector.ts:531-536) keeps the selection while it still matches and only otherwise falls back to the newest match BELOW it, then to the newest match of all — its own comment on the neighbouring `#closeFilter` is "Leave the filter, keeping the selected item outlined in the full transcript". Enter-in-filter inherited the same jump and now only requeries; backspace now requeries too, where it previously did nothing to the cursor at all. One helper (`requery_rewind_after_filter_edit`) is now the single place the cursor is decided, called from all three edit routes. Regression test `typing_a_filter_while_sitting_on_a_match_keeps_the_cursor` with a five-turn fixture, mutation-proven twice over: reverting to `matches.first()` fails with "the newest match below it — 3, not the oldest match at 0" (got 0), and deleting the keep-if-matching branch fails with the matching cursor stranded on a non-match.

- **iter-1619 — the rewind selector's live-jump could park the cursor on a turn compaction folded out of the transcript.** Found in the red-team pass over iter-1601's diff, after the live repro had already cleared the operator's symptom: the filter predicate existed twice on adjacent paths and the two disagreed. `filtered_indices()` — which `key_handling.rs:2517` and `:2543` call on every keystroke to jump the cursor onto the first match — returned every text match, compacted or not; `step()`, which the arrow keys use, filtered navigable entries first. So a query matching only a folded turn moved the cursor onto it: no rows, an outline over nothing, and an Enter that could only refuse — while the arrow keys, honouring the compaction gate, could not then reach it at all. Now there is ONE definition (`matching_indices`: navigable entries that pass the filter) and `filtered_indices` delegates to it, so the typed filter and the keys can never disagree. Regression test `typing_a_filter_that_only_matches_a_folded_turn_does_not_land_on_it`, mutation-proven by dropping the compaction gate from the shared predicate.

- **iter-1601 — the rewind selector's viewport never followed the outline, so Up/Down walked it off the top of the pane.** The operator reported the same symptom a third time, live: "the rewind preview is still very glitchy, and the arrow keys are not navigating". Two earlier fixes had already closed the zone boundaries (iter-1589) and the `sync_marks` cursor reset (iter-1515), and both were real — but neither decided whether an arrow press is *visible*. oh-my-pi does not model "where the selector looks" as operator state at all: its frame carries an anchor (`selStart..selEnd`, rewind-selector.ts:989-997) and `TranscriptBrowser::render` calls `revealRange` on every paint (transcript-browser.ts:232), which scrolls only when the anchor leaves the viewport and then clears follow-tail. We kept the scroll state and none of the follow — `scroll` was written ONLY by PageUp/PageDown (key_handling.rs:2556-2561) — so stepping up through a transcript taller than the pane moved the reversed outline above the top edge. The operator saw the tail unmoved while only the footer's "drops N turns" counter changed, which reads as dead keys. `reveal_offset` now implements omp's `alignment: "nearest"` rule (scroll-view.ts:331-345) as a pure function of the committed offset, the resolved zone, and the body height: pure, so a paint with no input cannot disagree with its predecessor (no oscillation, hence no flicker — the other half of "glitchy"), and an operator who pages away keeps their page. Two further divergences closed in the same pass: the footer now leads with omp's position counter (`turn N/M`), the cheapest honest readout to correlate against a keypress; and the arrow keys now skip a turn compaction folded out of the transcript (omp's `#move` skips `outlineVisibility` failures — a compacted turn keeps its mark so Enter can refuse it, but it has no rows, so parking on it drew an outline over nothing). While the filter is open the arrows walk the MATCHES (omp's `#stepFiltered`), where they previously landed in the dead `_ => {}` arm and a filtered list could not be moved at all. Five regression tests, each mutation-proven: disabling the reveal fails with "the outlined turn (prompt number 6) is off screen after Up — the pane still shows the tail"; dropping the compaction skip and dropping filter-awareness each fail on their own test.

- **iter-1589 — the rewind selector's preview painted the selected turn as EMPTY and its own answer as dropped.** The operator reported it live: "the entire preview is broken — it shows overlapping text from both the panes, while up/dn arrows do nothing to navigating". The arrows were the iter-1515 `sync_marks` defect (already fixed, pinned again below); the preview had never been asserted anywhere, and its root cause was the PICK/DROP zone. A turn is the prompt **plus everything the session produced in response** — prose, tool calls, tool results — so its block runs from its prompt row to the NEXT prompt's row. The renderer instead ended the block at `wrapped_user_prompt_ends[selected]`, which is the end of the **prompt row alone**. Measured on a seeded transcript (turn 3 selected, 120x45): the reversed zone was exactly ONE row (`3› third prompt`), and rows 29-34 — that same turn's answer — carried `DIM`, i.e. were painted as content the rewind drops. A highlighted row sitting directly on a dimmed block is what reads as "overlapping text from both panes" in a real terminal, where the two styles bleed at the row boundary. `rewind_selector.rs` now computes `sel_end` as the next prompt's start (or the end of the body), REVERSES the whole block, and DIMS only what follows it — oh-my-pi's exact three-zone rule (`#markRun`, `rewind-selector.ts:1066-1075`: `!target || i < start ? undefined : i < end ? "pick" : "drop"`). It also gains omp's caption over the picked turn — "Rewind to here · the prompt returns to the composer · N turns below dropped" — because a highlighted row with no verb attached left the one behaviour that cannot be guessed. Verified by three new zone tests and mutation-proven: restoring the old boundary fails them with `the outline must cover the whole turn, not one row — got 1`.

- **iter-1586 — Double-Esc to rewind opened nothing over a half-written draft, which is the one state the gesture is for.** The arm in `key_handling.rs` gated on `prompt_input.is_empty()`, so a second Esc inside the 500 ms window opened the selector only with an EMPTY composer — live-reproduced in tmux before the fix (typed a draft, pressed Esc Esc, nothing opened; Ctrl+U to clear, same two keys, selector opened). The selector means "go back a turn", and the operator who most needs it is holding text they are about to abandon anyway, so the gate made the reported feature dead precisely where it matters, with no advertised way forward. oh-my-pi gates the same way (`input-controller.ts:628-630`, "Esc must not destroy an in-progress draft"), which is why this is now a deliberate divergence in the operator's favour: rather than refuse, the draft is **parked** for the selector's lifetime and handed back on every exit. `App::rewind_draft_held` is a dedicated slot (not the Ctrl+S stash, which has its own bring-it-back chord), `park_composer_for_rewind` / `close_rewind_selector` / `restore_rewind_draft` are the three-move seam in `turn_state.rs`, and cancel restores the text *and the caret* rather than clamping to the end. A **committed** rewind discards the parked draft instead, because the rewound prompt takes the composer — a half-written sentence must not clobber the prompt the operator asked to go back to. Single-Esc behaviour is unchanged: the first Esc still only arms the clock. Verified live in tmux (selector opens over a draft, Esc hands the draft back verbatim), by two regression tests proven to fail without the fix, and by a new scenario golden in the corpus.

- **iter-1499 — a model-role vocabulary both surfaces share, and the TUI picker's four visual defects closed.** Oh-my-pi's picker is not a better list, it is a *role* model: named slots (`default`, `smol`, `slow`, …) that you address by word, with `default` the only one that switches the conversation. Operant already had the slots and no surface that named them. One new resolver, `operant-core/src/model_roles.rs`, is now the single seam: `ModelSlot` (Conversation, Fallback, Recap, Vision, Compression, Memory, MoaAggregator, MoaReference), `resolve_selection(word, cfg, effort)`, `slot_summaries`/`describe_slots`, `persist_key`, and the aliases operant's own config documents — `smol` → `[recap]`, `slow` → memory — resolution order is role name, then an exact assigned id, then a bare direct conversation id (a bare id is NOT refused: that is the command's actual use), and `exactly_one_slot_switches_the_conversation` is pinned by a test because the whole design rests on it. The **gateway `/model`** now prints a role catalogue on a bare call instead of nothing, resolves a role or a `provider/model` id through the resolver, bootstraps a session when one is missing (the RT-175 create-or-find fix `/goal` already had, without which a fresh chat's override silently evaporated), and finally parses the `--provider` flag its own `args_hint` advertised while never reading. The **TUI picker** pins one row per assigned slot above the endpoint catalogue and routes its selection through the same resolver, so `/model slow` on Telegram and Ctrl+O in the picker land on the same model; `Ctrl+O`/`Ctrl+N`/`Shift+Tab` were advertised in the footer and had no handlers at all, and now set the default, star a favourite, and cycle to the next starred model, both persisted.

- **iter-1499 — four picker defects, each a way the modal lied to the operator.** (1) While the endpoint catalogue was still arriving the picker showed a single configured row as if that were the whole list; the picker now carries `loading` and the hint says "listing models…" until the fetch drains (or fails). (2) The footer's keybind hint was 92 characters wide in an 80-column frame, so it was clipped mid-word into "Ctrl+N favorite · S" — the hint is now tiered by width (full / short / none) so a narrow terminal drops the chords rather than truncating them into a binding that does not exist. (3) The fetch drain called `picker.filter.clear()`, wiping the filter an operator had typed while the request was in flight; the filter is now preserved and reapplied to the rows that land. (4) The three advertised chords above existed only as text.

- **iter-1948 — The settings screen's numeric rows silently discarded an unreadable value — the edit was accepted and nothing took effect (found 2026-11-09) (RT-415).** **CLOSED iter-1948** — the edit buffer accepts ANY character (`KeyCode::Char(c) => push(c)`), and all three Number rows (Max Tokens, Auto-compact threshold, File autocomplete limit) resolved with `if let Ok(n) = value.parse()`, so typing `abc` and pressing Enter left the config field, the displayed value and the pending change all untouched with no feedback at all. Same class as `/rewind`'s silently discarded `[N]` (iter-1947), on a different surface. **Fixed**: `apply_and_save` now collects the unread rows and reports them, naming the row and the value the operator typed, and the footer IS the error while one stands (a state-gated footer). **Pinned** by `an_unreadable_number_says_so_instead_of_saving_silently`, whose failure caught a second defect in my own first draft — the failed entries were returned before `pending_changes.clear()`, so every later save would re-report a value already reported.

- **iter-1949 — The advertised-argument class is now swept and closed — `/copy`, `/rewind` and the settings screen's three numeric rows all fail loud (verified 2026-11-10) (RT-432).** **CLOSED iter-1949** — the class rule iter-1947 established (an input a surface advertises must fail loud when it cannot be read, never silently fall back) has now been run over every reachable member: `/copy` (iter-1772's exemplar), `/rewind` (RT-409, iter-1947), and the settings screen's Max Tokens / Auto-compact threshold / File autocomplete limit rows (RT-415, iter-1948). The remaining `parse` sites in the TUI are all internal data — `UnicodeWidthChar::width().unwrap_or(0)`, markdown and diff layout offsets, JSON fields, terminal size, git output — none of which parses operator-advertised input. **Recorded**: the Enum and Bool arms of `apply_and_save` are unreachable (see the detail) — dead code, mentioned per CLAUDE.md, not deleted.

- **iter-1950 — The session hub's `o` sort key was a FALSE finding of mine — the arm existed, my grep range stopped short of it; the class now has a pin that would have caught a real one (found 2026-11-10) (RT-439).** **CLOSED iter-1950** — the hub's header advertises `Group: <mode>  g` and its tab bar advertises `Sort: <mode>  o`, both from the state's own `key()` accessors, so I swept the dispatcher for the arms and found none for `o` in the range I read. I 'fixed' it by adding a duplicate arm. On re-reading the whole block the REAL arm was there all along at `key_handling.rs:843`, and the mutation-proof run showed it. **Fixed** by REVERTING my production change — the duplicate was removed and `Char('o') => cycle_sort` is back to exactly one occurrence. **Pinned**: the class now has `the_session_hub_advertises_only_keys_that_cycle_something`, which drives the REAL handler for every key a hub affordance names and requires the mode to land one step along its own ladder. It would have gone red on a genuinely missing arm.

- **iter-1951 — The pinned-diagram pane is an UNWIRED SEAM: its renderer paints a whole keyboard contract from a function that never runs (found 2026-11-10) (RT-440).** **CLOSED iter-1951** — `operant_ui::draw` derives the pane's candidates from a hardcoded `Vec::new()` (marked `[port-decision]` — "wire at operant_app::mermaid::get_active_diagrams", which has ZERO callers), so `diagrams.len()` is 0, `pinned_diagram` is `None`, and `draw_pinned_diagram` — whose title paints ` o open`, ` hide`, ` ⇄ side`, ` Ctrl+←/→` and ` Ctrl+H/L focus` — is never called. **Classified as a deliberate port-decision stub, NOT a false claim**: unlike iter-1944's `/branch N`, no operator can reach this copy, so it is not a lie the operator reads. What was missing was any proof the seam is closed. **Fixed**: `the_pinned_diagram_seam_is_closed_and_its_painter_never_runs` drives mermaid through the REAL full-frame renderer and asserts none of that contract paints; it is mutation-proved by opening the seam (a real `DiagramInfo`, mode `Pinned`, pane enabled), which reddens `the pinned-diagram renderer must not paint`.

- **iter-1952 — `/settings`' "Show current directory" and "Show git branch" rows were WRITE-ONLY — the header read nothing (found 2026-11-10) (RT-443).** **CLOSED iter-1952** — `TuiState::show_working_dir()` and `show_branch()` defaulted to `true` in the trait, and `impl TuiState for App` OVERRODE NOTHING, so the header's new gate (`app.show_working_dir()`, wired at the peer's iter-1897) returned a constant. `Settings::show_working_dir()` — the consumer method, written in the same iteration as the row — had NO caller between the row and the paint. Turning either flag off persisted and displayed correctly and changed NOTHING on screen. **Fixed**: `impl TuiState for App` now reads `self.settings.show_working_dir()` and `self.settings.show_branch()`. The peer's own iter-1898 test (`the_headers_directory_row_obeys_the_settings_display_flags`) had been red for exactly this reason and is green.

- **iter-1953 — `/settings` offered "Auto-compact threshold" whose value changed nothing — a PHANTOM ROW, removed (found 2026-11-10) (RT-446).** **CLOSED iter-1953** — the row advertised "Context usage % at which to trigger auto-compact (0-100)", but the TUI's own warnings fire at hardcoded 100/95/80 (`check_token_warnings`) and no path under `tui/` reads `config.agent.context_compression_threshold`. The value was also NOT persisted: `apply_and_save` writes it onto the in-memory `AppConfig` and then saves `Settings` (settings.json), which has no such field. Per the audit's phantom-capability rule the CLAIM is the defect, so the claim goes, not the feature. The live readers (`main.rs`'s `LlmCompressorConfig`, `cmd_setup`) are untouched.

- **iter-1954 — `/settings`' "Max Tokens" row did not survive a restart — the SAVE half was missing while the consumer half worked (found 2026-11-10) (RT-450).** **CLOSED iter-1954** — round 69 found the consumer missing; this is the mirror. `apply_and_save` set `config.agent.context_window`, which `check_token_warnings` reads live, and copied the value into `InnerConfig.max_tokens` in settings.json, which NOTHING reads back, and the loaded `operant.toml` was never written. So an operator set Max Tokens, saw the footer agree, restarted, and silently got the shipped default back. **Fixed**: `persist_config_key` — the one surgical writer the repo already trusts with `agent.model` — now writes `agent.context_window`, applied FIRST and reported, not gated.

- **iter-1955 — The stall watchdog is a PARTIAL PORT: its doc header promises liveness heartbeats and stall dumps, and the ported file contains no threshold, no interval, and neither log event (found 2026-11-10) (RT-467).** **OPEN — inventory, no assertion warranted** — `operant_app/logging/watchdog.rs` landed verbatim from jcode with the bookkeeping primitives (`beat`, `begin_work`, `is_busy`, `resource_snapshot`, `thread_states`) but WITHOUT the monitor thread, its timer, or the thresholds capabilities 2 and 3 compare against. The whole module has ZERO callers outside its own tests, so `LAST_BEAT_MS` is written by `beat()` and read by nothing: a silent freeze leaves the log exactly as silent as the module exists to fix. Building it is a product decision, not an audit's.

- **iter-1956 — The right-click context menu offered a Copy it could not perform and failed SILENTLY when it could not (found 2026-11-10) (RT-472).** **CLOSED iter-1956** — `context_menu_items` returns `&[ContextMenuItem::Copy]` for BOTH kinds unconditionally, and `open_context_menu_at_cursor`'s last branch opens the menu with `ContextMenuKind::Selection` when there is no message at the row and no selection. Executing it walked `handle_context_menu_action`'s `if let Some(text)` guard and dropped out with NO notice: the operator picked a visible, executable menu item and saw nothing. **Fixed**: the let-else names what to do instead of no-opping.

- **iter-1957 — One chord in one context may carry exactly one claim — the registry had no machine check for it and the class now has one (found 2026-11-10) (RT-478).** **CLOSED iter-1957** — a class sweep of the catalogue found no live defect, but `register` PUSHES and `find` returns the FIRST match, so two entries for one chord in one context leave the second unreachable while `/keys` and `/hotkeys` render BOTH: the operator sees one chord with two promised effects and usually gets one. The vim sweep's scope note asserts "the registry has no unreachable duplicate" for its own contexts WITHOUT a machine check. This round adds it for every context.

- **iter-2077 — Runtime model overrides were discarded when same-provider fallback was enabled.** The agent correctly built each request with `effective_model()`, but `FallbackModelClient` replaced that field with the startup primary before the first wire call, so `/model` and per-session overrides silently selected the configured model again. **Fixed**: the request's model is now the per-request primary, followed by the configured fallback list; logging and fallback events name the same primary. A behavioral regression records the actual model sent, and restoring the startup-primary behavior makes it fail.

- **iter-2088 — Background review timing, routing, and scope were incomplete.** The memory lane spawned mid-turn, auxiliary routes ignored their endpoint and credential, several skill actions bypassed the review guard, and read marks leaked across concurrent reviews. **Fixed**: both review lanes now spawn once at the terminal exit; model/endpoint/credential resolve as one auxiliary route contract; every mutating skill action is scope- and read-guarded with task-local read marks; and the memory prompt now reports candidates instead of attempting a forbidden write. The timing, routing, and guard regressions are mutation-proved.

- **iter-2095 — Five remaining CLI rustdoc links did not resolve.** The strict documentation gate therefore failed even though the code compiled. **Fixed**: the labels are plain code spans rather than false intra-doc links. **Pinned** by the rustdoc gate.

- **iter-2094 — The hermetic self-test could start with an empty Cargo registry cache.** The offline mode was correct for the gate but made a fresh machine fail before compiling anything. **Fixed**: the self-test fetches the locked registry crates once, then runs every Cargo step offline. The dependency revision and the live operator state remain pinned.

- **iter-2093 — A fresh review found three boundary bugs in the previous fixes.** An auxiliary endpoint could inherit the primary credential; a process-global fallback could let one background review inherit another review's skill reads; and the broad healing dedup could suppress a legitimate repeated user turn. **Fixed**: endpoint overrides are keyless unless the slot supplies a credential, read marks exist only inside their review task, and dedup applies only to the immediate retry marked by the healing hint. Each regression is mutation-proved.

- **iter-2092 — The release gates exposed stale ledgers, missing baseline explanations, broken rustdoc links, and an outdated settings golden.** **Fixed**: every RT-488–RT-501 index row now has its detail record; the config and lint audits account for settings that already warn; rustdoc links resolve without private-item leakage; the settings corpus reflects the corrected **Context window** row; and the streaming path now has the same named repeated-overflow regression as the non-streaming path.

- **iter-2091 — The self-test was not hermetic.** It advanced the git dependency and rewrote the untracked lockfile, workspace tests inherited the operator's live Operant home and credentials, the TUI corpus reached the updater/models.dev/AFT network paths, a failed build left later gates testing a stale binary, and the advisory gate could fetch a changing database. **Fixed**: the dependency revision, store, config, and credentials are pinned; the corpus disables updater/AFT and seeds a local catalog cache; binary-dependent gates refuse stale artifacts; advisory fetching is opt-in; and the script is CWD-independent. The supply-chain contract now covers the no-fetch path.

- **iter-2090 — A self-healing retry could persist the same user message twice.** The retry inserted a system recovery hint after the original user row, so the next turn's dedup checked only the final message and missed the user row beneath it. **Fixed**: dedup now compares the most recent user message across intervening recovery hints. The persistence regression is mutation-proved.

- **iter-2089 — ACP and delegated/custom-core agents ignored configured context caps.** **Fixed**: core constructors seed their caps from the installed operator configuration, ACP explicitly applies the configured caps and compressor, and CLI construction reuses one shared mapping. The regression proves a configured 2k cap compacts an approximately 5k history without a separate builder call.

- **iter-2087 — Metered work could disappear or be charged to the wrong route.** A failed stream discarded token usage it had already received, and cost events always named the startup model even when a fallback route served the request. **Fixed**: streaming chunks carry the provider-reported model, failed streams emit their known usage before surfacing the error, and usage/cost attribution uses the route that actually served the request. Both regressions are mutation-proved.

- **iter-2086 — Cross-provider fallback stopped after one alternate provider and leaked its route into later requests.** **Fixed**: a provider-auth/rate-limit failure now walks every configured fallback route until one succeeds, and the route choice is request-local rather than mutating the registry's active cursor. Deep-walk and request-local regressions are mutation-proved.

- **iter-2085 — Context-limit learning and repeated overflow recovery were untruthful.** A rejected request's estimated size was persisted as if it were the provider's context limit, and a second overflow escaped as a raw provider error whenever compression reduced the foldable history. **Fixed**: explicit provider limits are learned directly, unknown limits probe the next smaller tier below the rejected request, learned limits remain monotonic, and every second classified overflow fails with the typed `ContextExhausted` error. The probe and repeated-overflow regressions are mutation-proved.

- **iter-2084 — Streaming recovery had two transport-truth defects.** A streaming HTTP 429 returned immediately instead of honoring `Retry-After` and the configured retry budget, and a prior credential rotation disabled unrelated mid-stream transport retries. **Fixed**: the streaming client now drains the rate-limit bucket and retries a 429 after the provider's `Retry-After` (or exponential backoff); stream recovery no longer depends on the credential-rotation guard. Both regressions are mutation-proved.

- **iter-2081 — The self-test's "quick run" contacted the operator's live provider and could mutate the real runtime tree.** The command inherited the operator's config, credentials, and `~/.operant`, so a gate that claimed to test the product instead exercised a live account and wrote real runtime state. **Fixed**: the self-test now runs a credential-free local OpenAI-compatible provider over HTTP and SSE, drives a real `verify_task` tool call, connects a local MCP stdio server, and uses a throwaway `HOME`/`HERMES_HOME`; hashes of the real operator configuration must remain unchanged. Focused gateway and TUI regressions also pin foldable-context compaction/retry and protected-prefix error guidance.

- **iter-2080 — Verification could report success without running tests and could leave start descendants alive.** A recipe with no test phase made `phases.iter().all(...)` return true, so a build-only run reported `ok=true`; the start phase also killed only its shell, leaving background descendants running after readiness timeout. **Fixed**: a successful verification now requires a test phase, and the start command runs in its own process group that is terminated on readiness or timeout. Behavioral regressions observe the false-green verdict and the descendant PID; mutation-proving either old behavior makes its regression fail.

- **iter-2079 — A bounded session rehydration could cut through a tool-call group.** The newest count/byte window selected raw rows, so a resumed request could begin with tool results whose assistant request was outside the window; the later orphan repair then dropped those results and stripped their matching calls, losing tool history. **Fixed**: the rehydration window is aligned to complete assistant/request/result groups, keeping either the whole group or none of it. Behavioral regressions cover both a partial group and a complete tail group; restoring the raw row cut makes the partial-group regression fail.

- **iter-2078 — Grace and circuit-breaker replies skipped the normal turn finalization.** A breaker abort returned immediately after its `Done` event, and a grace summary emitted only its end notices, so neither path synced the terminal reply to memory, queued the next prefetch, fired the `TurnEnd` lifecycle hook, or emitted the `TurnEnd` bus event promised by the architecture. **Fixed**: normal text, grace, and both circuit-breaker exits now share one terminal finalizer, with the obligations running exactly once. Behavioral regressions observe memory, prefetch, the lifecycle hook, and the bus on both non-text exits; removing the breaker finalizer makes them fail.

- **iter-2075 — The context-window failure was a CONFIG-WRITER INCIDENT, not a conversation that needed folding.** A settings test persisted its fixture value into the operator's live `operant.toml` while a read-only skill expansion was reloading the live config and redirecting the process-global surgical writer. The resulting 4,242-token window was smaller than the protected 9,711-token prompt, so `/compact` could never help; every error nevertheless told the operator to compact. **Fixed at the root**: config validation no longer installs the writer target, skill preprocessing reads only the installed runtime config, and the settings row persists to the config's explicit source path. `agent.context_window` is now named and validated as total prompt+response capacity (the runtime reserves 4,096 tokens for the response), the TUI row is honestly named **Context window**, and exhaustion distinguishes foldable history from an impossible protected prompt. The latter names the required and configured token counts, says to increase `agent.context_window`, and the gateway no longer performs a futile compact-and-retry. Regression coverage: explicit/read-only writer isolation, impossible-window config and TUI rejection, protected-prefix floor, and both actionable error messages.

- **iter-1958 — The catalogue was never checked BEHAVIOURALLY: an advertised chord that dispatches nothing types itself, and nothing tested that (found 2026-11-10) (RT-484).** **CLOSED iter-1958** — round 74 closed the duplicate-chord class; this closes the named residual "per-dialog keybinding catalogue contexts" the PROSE way it was recorded. Both existing pins (`global_bindings_should_be_dispatched_in_key_handling`, its modifier half, and `every_string_dispatched_vim_binding_is_reachable`) read the SOURCE, and their own doc comments name the limit: "a bare `x` arm can satisfy a chord advertised as `Ctrl+X`". `no_catalogued_chord_types_itself_into_the_composer` asks the CODE instead — the composer inserts an unhandled key's character, so an advertised chord that consumes nothing is observable without knowing anything about dispatch.

- **iter-1959 — The catalogue's single Dialog promise ("Esc closes a dialog") was never tested against the surfaces it describes (found 2026-11-10) (RT-486).** **CLOSED iter-1959** — `key_handling.rs` names `KeyCode::Esc` in 38 arms, so the promise is plausible, but a chord being somewhere in the file is not the same claim as it being on every surface. `every_modal_dialog_closes_on_the_escape_the_catalogue_promises` opens the modal dialogs and drives the one key the catalogue promises, requiring each to close.

- **iter-1960 — The catalogue's single Dialog promise was tested on 5 of the 11 modal surfaces that describe it (found 2026-11-10) (RT-487).** **CLOSED iter-1960** — round 76's pin covered `stats`, `export`, `free_mode`, `key_input` and `custom_provider`, and left `bypass_permissions`, `connect`, `import_config`, `ask_user`, `help` and `history_search` untested. All six also expose the same `visible: bool` the renderer reads and are all gated in `key_handling.rs`, so the gap was the pin's coverage, not the code's.

- **iter-1961 — Every chord the Completion context describes must do the thing its description says, and nothing tested that (found 2026-11-11) (RT-506).** **CLOSED iter-1961** — round 75's behavioural sweep covers Completion, but its signal is "the key typed itself": it catches a chord nothing consumed, and cannot distinguish a chord that swallows the key from one that does what it says. Completion is exactly that risk — `Down`, `Up`, `Tab`, `Enter` and `Esc` are advertised as motion and acceptance, and an arrow that consumed the key while doing nothing would round 75 green and the operator stuck. **Fixed as a test**, not a repair.

- **INVENTORY — `make_app()` is not isolated: a process-global left over from an earlier keypress makes the same chord behave differently in a later fresh app, so any cross-app sweep is unsound (found 2026-11-12) (RT-527).** **OPEN — INVENTORY** — a Prompt-context sweep I wrote this round reported `Alt+B`/`Alt+D` as silent no-ops, and both work in isolation. The poison is a process-global written while an earlier binding was being driven, not the app: within one app `Alt+B` moves the cursor correctly across a typed character.
- **iter-1962 — `Ctrl+Y` ("Paste") consumed the key and did nothing when the kill ring was empty (found 2026-11-12) (RT-528).** **CLOSED iter-1962** — `yank()` returned nothing at all, so with an empty kill ring neither the composer nor a status notice moved: the affordance `/keys` advertised for Prompt context silently did nothing. The dialog paste path already said "Clipboard is empty", so the composer path was the odd one out.

- **iter-1963 — `Ctrl+V` ("Enter Visual Block mode") is advertised and implemented, but nothing delivers the keystroke — the mechanism is unreachable and a comment claims it already works (found 2026-11-12) (RT-529).** **CLOSED iter-1963** — `vim_command.rs:340` matches `key == "\x16"` in Normal mode and enters VisualBlock, and `keybindings/defaults.rs` advertises the chord. But no `if` anywhere in the crate produced the `"\x16"` string: the text-entry arm excludes CONTROL letters by design (so a chord can never degrade into its bare letter), and no dedicated arm carried it. A comment on the clipboard arm even claimed "\x16 is already consumed by the vim handler above".


### Fixed

- **iter-1495 — the session hub no longer leaves the idle animation running underneath it, and subagent rows stop inventing usage they never measured.** Two rendering defects, both found by a red-team pass over the previous round rather than by a user report. (1) The decorative idle animation was gated by an atomic flag that **no code ever wrote** — the intended "the hub is open, pause" call did not exist, and the diagnosis was also wrong: `render_app` paints the chrome and *then* the overlay ladder paints the hub over the whole frame, so the animation was never visible under the hub. The real cost was a 14-row layout reservation and a full 3D sample on every frame the hub sat open. The gate now lives where the reservation happens (`should_reserve_idle_animation`, reading `TuiState::session_hub_visible()`), so the frames stop being spent. (2) A subagent row rendered `$0.0000 · 0 msg · 0.000k tok`, which asserts the delegation registry measured usage and found nothing — it measures **none at all** (`async_delegation::records_snapshot` hardcodes all three zeros). Following oh-my-pi's roster rule that missing data reads `usage —` rather than an estimate, the row now prints the dash and, in place of the fake numbers, the one fact it did have: the delegation status (`pending`/`completed`/`failed`), which previously existed only in the focused preview so a Completed and a Failed subagent looked identical in the list. Ordinary session rows keep their real numbers — for sessions those are measured and true.

- **iter-1495 — a subagent row draws its badge once, not twice.** `row_title` already prefixes a subagent row with 🤖, and a second badge span was added in the list renderer, so the drawn row read 🤖🤖 with a stutter between the glyphs. The duplicate is gone; what the row was actually missing was colour, so a subagent title now takes the accent colour.

### Docs

- **iter-1492 — pre-upgrade survey: the oh-my-pi model-picker's role nuance, operant's own picker, and a 4-step /ponytail plan (`docs/plans/plan-2026-10-30-model-picker-roles-upgrade-2026-.md`).** The reference (`../external-repos/oh-my-pi`) does not have a better picker, it has a *role* model: 11 chat roles (`default`, `smol`, `slow`, …), each a named slot with a ~38-candidate priority chain, a `:level` thinking suffix and an `@role` alias — and `default` is the only role that switches the conversation, so every other assignment is silent. Operant's analogue is already partly live and no surface exposes it: `agent.model` + `agent.fallback_models` (live), `auxiliary_models.{vision,compression,memory}` (live), `[recap]` — which `config.rs:70-75` itself calls "the omp `smol` role" — `[moa]`, `[advisor]`, plus six slots `config.rs:2877-2905` already warns are inert. The TUI picker is `open_model_picker_inline` (`tui/app/providers.rs:102`), the endpoint-rooted three-column inline picker that replaced the deleted `model_picker/` overlay; its `PickerEntry` already carries `is_default`/`is_current`/`effort` and drains into `pending_model`/`pending_effort`. The survey records the gateway's three real gaps: `/model` prints no catalogue at all, its sync arm has no `create_session` (the RT-175 bootstrap `/goal` got, so on a fresh chat the override silently evaporates), and `--provider` is declared in `args_hint` but never parsed. Slack/WhatsApp/Email/SMS cannot receive (Socket Mode, RT-86), so the work lands on Telegram+Discord+webhook. No code change — this iteration is the survey and the ranked plan.

### Removed

- **iter-1461 — the `session_branching` overlay is gone (audit R1: the re-architecture's deletion was half-done).** The unified-session plan deleted the old `session_browser` modal and left its twin standing: `session_branching.rs` was a second pane for browsing sessions (its own list, navigation, render, key arms, dialog priority, drain, loader and tests), showing exactly the parent + forks of one session — which the hub row already carried as `parent_id` + `RowKind::Branch`. Ctrl+B and `/branch` now open the hub on a new `CurrentBranch` filter (the root's row and its forks; the anchor is the live row when the load lands, so an empty hub never lies). Deleted: the overlay module (319 lines), `fork_tree` + its whole drain path (`branch_list_rx`, `pending_branch_list`, `drain_branch_list`), the dialog-priority row and enum variant, 11 key arms, 2 tests, and both corpus rows in `tui_scenarios` (`schema.json`, `_validate.py`) — which also removed a stale `session_browser` entry that had survived the earlier deletion. The two write arms became documented routes instead of dead UI: forking is `/rewind`, deleting is `operant sessions delete`. The hub gains Home/End jumps and wheel-scrolls its preview (audit R4/R5 — the picker parity items jcode has and this pane lacked).

### Added

- **iter-1461 — prompt-jump chords (jcode's highest-value shortcut gap, closed).** Alt+[ / Alt+] jump the transcript viewport to the previous / next USER prompt — the question you asked, which is the mental model an operator navigates a long session by. Positions come from the frame the renderer last drew (`wrapped_user_prompt_starts`), the same seam jcode reads; the jump pauses auto-follow, each press walks one prompt in document order, and past the last prompt Alt+] hands the view back to following. The status row names the landed prompt and both directions, so the chords are discoverable without the help overlay. Deliberate port-correction the audit owed a reader: **Ctrl+[ is byte 0x1b — the Esc key** — so it arrives as Esc+CONTROL under the kitty protocol, as Char('[')+CONTROL under tmux extended-keys, and as Char('5')+CONTROL (a different chord) in the legacy raw path; the shipped primaries are therefore Alt+[ / Alt+] (also jcode's own defaults, and portable in every mode), with the ctrl spellings dispatched as the same chord where the terminal reports them. The load-bearing half of this iteration is not the chords but three Esc consumers that gained `KeyModifiers::NONE` guards — streaming-cancel, double-Esc-rewind, and transcript-focus each accepted Esc with any modifiers and would have swallowed the jump (a mid-stream Ctrl+[ would have cancelled the running turn; the transcript-focus arm would have silently switched focus). Seven mutation proofs, 10 regression tests, full bin sweep 1860 green, deployed and three-way md5-verified against the running gateway.
- **iter-1476 — the session hub's preview pane and its navigation work (audit 2026-10-29 D1/D2/D4/D5).** The operator reported four live defects at once: an empty preview, Tab and PageUp doing nothing, a stuck “Loading session…” that never resolved, and a preview showing only the last message. Three had one root cause each and one had none. The preview now renders one row per stored message of the selected session — role label, flattened text, tool-call labels parsed off the stored JSON, and a placeholder row rather than a blank line for a message with no visible text — loaded on a blocking thread over its own channel so the list stays interactive while the pane fills, with the row id riding inside the message so a load that lands after the operator moved on is DROPPED rather than painted. `PaneFocus` (jcode's `session_picker/navigation.rs` model) is what makes the step keys work at all: Tab toggles it, `h`/`l` set it explicitly, and Up/Down/j/k, PageUp/PageDown/K/J and Home/End route to the FOCUSED pane, so the same four keys move the selection in the list and scroll the transcript in the preview. `Right` nudges the preview into focus as the discoverable way in; `Left` deliberately keeps operant's omp close-tap rather than jcode's pane-focus (recorded so it is not “fixed” into a regression). The focused pane carries an accent border and a title naming what Tab moves to, and preview scroll clamps to a bound the renderer measures each frame and reports back through the frame ctx — arrow steps cannot scroll into blank space. The stuck loading notice is gone: with nothing cached and nothing in flight the pane now says “No transcript cached — press r to load”, and `r` is wired in both panes. The fourth defect's fix is in core, not TUI: `user_turns` is a process-global list that only ever grows, so a session switched INTO had either no marks (its turns happened in an earlier process) or a stranger's, and the rewind selector navigated a list that did not describe the loaded conversation — `set_session_id` now re-derives the loaded conversation's marks from its rehydrated transcript, which also keeps the fork path's post-rewind list exact (the fork persists its prefix before retargeting). Six mutation proofs (each revert → red → restore byte-identical), 2 new core tests, 14 new hub tests, clippy gate green, bin sweep 1880/0. The pre-existing `conversation_scoped_marks_exclude_other_sessions_turns` expectation is inverted to assert the fixed behaviour rather than the defect it used to pin. Honestly NOT fixed and recorded with mechanism: the hub still opens its own `Database::init` per open (measured 303 ms for `list_sessions_rich(500)` on a 105 MB DB) — threading a shared read-only connection through `App` is a whole-boot-path change the reported symptoms did not justify; and plan items 2-5 of the dated-frozen selector plan (sibling branches, prompt-text filtering, tail cap, in-transcript entry, store-stat surface) remain open.



- **iters 1444/1450/1458/1459 — unified session management across every surface (plan 2026-10-28 v2: jcode picker geometry, omp's AgentHub model, employee tags).** One session record — `SessionEntryV2` (`id`, `parent_id`, `title`, `updated_at`, `employee_id`, `source`, `message_count`, `cost_usd`, `tokens`, `cwd`, `status`, `kind`) — now backs the TUI, the CLI, and the daemon's API. The gateway store's BINDINGS (platform, platform user, channel) are a separate thing and say so: `operant gateway sessions` prints those maps and names itself "Gateway Session Bindings" rather than looking like a second session browser (audit R2). The TUI's 70×20 `session_browser` modal is gone, replaced by the full-frame **session hub** (`/session`, `/resume`, `/sessions`, or double-tap ← on an empty composer): jcode's 40/60 list+preview layout, per-character search highlight, filter cycle (All / Current dir / By employee / Active / Archived), AND-token search across every field, live subagent rows as peer rows (omp's model — main and delegated workers are one record kind), Enter to switch sessions or focus a subagent, ←← or Esc to leave. The ←← gesture honors omp's window exactly: two Lefts inside `[40ms, 500ms)` open the hub armed, so ONE further ← closes it (the close-tap handoff); a sub-40ms burst is key-repeat and never fires. The CLI's List/Browse/Prune read the same loader (`list_sessions_rich`, 500-row default — the 50-row cap is gone). Sessions record the agent's seat as `employee_id` (the operator's “who is executing” tag; NULL honestly for rows that predate seats — no backfill lie). And the gateway daemon serves the record read-only over HTTP on `127.0.0.1:8731` — `GET /api/sessions`, `GET /api/sessions/{id}/messages?cap=N`, `GET /api/subagents` — token-gated with the dashboard's auth discipline (`[gateway] api_enabled`, default on), so scripts and the dashboard page can read what the picker shows without opening the database.

### Changed

- **iter-1457 — the advisor's notes reach the acting agent (omp `resolveAdvisorDeliveryChannel` parity).** The advisor was a display-only watchdog: notes rendered as TUI cards and nothing else, while the module doc claimed omp itself never lets advisor output reach the model — a misreading. omp's advisor notes ARE model-visible (that is the feature: the primary acts on the advice); its guards (the `<advisory guidance="weigh, don't blindly obey">` framing, the boundary-guidance header, hazard quarantine) exist precisely because the notes enter model context. This iteration ports the delivery policy: the note contract now carries severity (`- [concern] …`, omp's nit/concern/blocker; a bare dash is a nit), one merged review delivers per turn (never N cards for N notes), and the channel decision follows `[advisor] continue_severity` (default `concern`, omp's `continueSeverity` default): a note at/above it submits the batch — boundary guidance + per-note `<advisory severity="…">` elements, XML-escaped — as the next turn's query so the primary sees and acts on it; lower severities preserve as a card that rides the transcript into the next user turn. The continuation query is tagged `[ADVISOR_CONTINUATION]`, the engine's latch skips reviews of tagged deltas (one batch can never cascade into two continuations — the omp continuation latch), a user interrupt drops any queued continuation (user-stop suppression), and a fresh user message supersedes a queued one (the operator is driving again). Mutation-proven at the three load-bearing sites: the channel-decision function, the latch predicate, and the TUI's marker extraction.

- **iter-1349 — the six inert `[auxiliary_models.*]` slots stop lying to the operator (audit 2026-10-26).** `web_extract`, `image_gen`, `embeddings`, `search`, `code_execution` and `reasoning` are parsed but read by nothing — `image_gen` reads the `FAL_KEY` env var instead, the live embeddings knob is `agent.context_lcm_embedding_model`, and web search is the sourcehound binary. They were worse than absent because `operant setup` interviewed the operator for them and a set value was silently ignored, which is a misconfiguration with no debugging path. Two sites: a set value now produces a per-slot `accepted but unread — nothing in the code routes to it` line from `AppConfig::unused_setting_warnings` (the same production load-time tripwire that already reports `[pk]`, `[tui]`, `[skills]`, `[bylaws]`, `[socialization]`, `[memory]` — those nine other inert keys needed no work at all, they were already honest), and the setup interview drops from 9 slots to the 3 that route (`vision`, `compression`, `memory`). Struct fields remain — removing one would break any existing `operant.toml` that sets it, because `AppConfig` carries `deny_unknown_fields`, so field deletion is a breaking-release cleanup. The regression test names each inert slot, proves a live slot stays quiet (a warning on a legitimately configured slot would train the operator to ignore the stream), and treats an empty model as unset; it is mutation-proven (fails `web_extract: []` when the flag is neutralized). `operant.example.toml` now documents which three slots are real and why the other six are not.
- **iter-1372 — three tool descriptions stopped promising things the tools do not do.** `aft_search` claimed a "lexical fallback when the semantic index is not enabled"; live aft returns a HARD `[search_lanes_unavailable]` with both lanes unobserved, so the description now states that verbatim and routes to `aft_grep` (which has a real fallback) instead. `process` "list" claimed to list all processes — it lists this agent's own spawned background sessions, so an empty list now reads as "nothing spawned" rather than an OS verdict. `aft_checkpoint` claimed "a snapshot of the project state" — it captures only git-tracked files, so the scope limit and the `aft_checkpoint_paths` preview are now named. All three pinned by contract tests that assert the substitute path too, so the honesty fix never strands the model.
- **iter-1372 — `tui_scenarios` verify is unblocked, and `at-file-ref-grammar` is executable for the first time.** It carried an illegal `<text>` key token (typed as seven literal characters), asserted a `prompt_input.suggestions_source_file_ref` field `debug_snapshot()` has never exposed, and had no baseline keys — the last of which is what killed `_run.py verify` as a bare `KeyError` for the whole corpus (RT-180). It now uses the corpus's existing `deterministic: false` + `nondeterminism` opt-out with an honest reason (the popup's contents are working-directory dependent; the unit layer is the grammar's real oracle), plus the legal `@cr` sequence and assertions the snapshot actually supports. `runs_of` also raises an actionable error naming the file and the fix instead of a `KeyError`.
- **iter-1373 — the memory bank boundary is written down (RT-190, option c ruled).** `operant.example.toml`'s `[memory]` section now states that operant's in-process store (`$OPERANT_HOME/memory_wire.sqlite`, bank `operant`) is a DIFFERENT store from a `memory-wire serve` daemon's (`~/.local/share/memory-wire/memory.db`, banks `omp`/`memory-wire`), that nothing crosses by default, and that unifying them is a planned tranche rather than a config flip.
- **iter-1379 — two functional-core commands finally have real channel twins (RT-179 plan item 2.3).** `/effort [off|minimal|low|medium|high|xhigh|status]` and `/plan [on|off|status]` now resolve as gateway commands instead of falling through to the model as literals. Before this, the TUI registry had 72 commands the gateway could not reach by name and these two were the ones with both real operator value AND an enforcement seam that already existed. `/effort` drives the agent's live reasoning-effort cell through the turn lock — the SAME `set_reasoning_effort` the TUI's picker drains into, so it is a genuine twin rather than a metadata echo (the R17 `/yolo` lesson: a key nothing reads is a lie). `/plan` gives the gateway a read-only posture it never had, mirroring the TUI's `PermissionMode::Plan`: gated tools are denied outright rather than prompted, checked BEFORE YOLO so a channel in both sets never widens, and a seat-policy escalation in plan mode is still refused with its audit row recorded (the F2 lesson). Both are in-memory like YOLO's set rather than persisted, because a read-only posture that silently survived a restart would surprise an operator who forgot it was on. Verified by five tests including a runtime-path test that READS the agent's cell back — removing the setter call fails it — and both mutations were proven.
- **iter-1380 — the ledger's remaining gaps get an execution outline, and two fleet incidents get their record (RG-014, RG-015).** The outline `docs/plans/2026-10-27-remaining-implementation-gaps-outline-execution-plan-for-the.md` is built from a fresh re-verification of all 7 open checkbox rows plus the three prose rows a checklist count misses (S2, D-5b, D-6), and it marks what already landed so nobody re-implements it: the peer's iter-1379 shipped RT-194 (23 sourcehound composites as first-class tools on the deferred bridge), and my iter-1379 shipped RT-179's first two twins. Each remaining row carries its verified state, its acceptance criterion, and an explicit "not scheduled" list (the TUI shell-outs, the operator's skills pool, and any row needing an owner ruling). RG-014 records that the iteration label is read-then-computed, so two agents in one push window publish the same number — the pre-push `ahead=1` check does not cover the label, only the commit count, and the fix is one extra re-fetch immediately before pushing. RG-015 records that an uncommitted peer edit reddens the shared tree and a red tree is not evidence a passing change broke: the attribution check is `git status --porcelain -- <failing path>` before treating a compile error as yours. RT-179's row is corrected to say its blocker closed at iter-1298, since a stale "blocked" sentence invites the next agent to re-plan around a blocker that no longer exists.
- **iter-1389 — the sourcehound group hint names the path that actually exists (RT-195 residual).** It told operators to edit `~/.config/sourcehound-mcp/settings.yml`, a directory that does not exist in the default layout — `sourcehound status` on the live 1.8.0 install prints `Config: ~/.config/sourcehound`, with `settings.yml` inside. The `-mcp` spelling is real in the binary's own strings, but it only materializes under a `SOURCEHOUND_CONFIG_DIR` redirect, so naming it as the default sent the operator looking for a file that was never there. The hint now leads with the live default path, names the redirect and `XDG_CONFIG_HOME`, and still carries the actionable half (add "web" to `tools.groups`). Pinned by `group_hint_names_the_path_the_live_install_actually_uses`, which fails if the path is reverted — mutation-proven.
- **iter-1389 — the WIDE (emoji) glyph class is cleared, on the terminal side (S2).** A live 200×50 tmux 3.8 pane driving a FRESH debug build of the real TUI rendered `A🐝B✅C⚠️D`, and the pane's right border landed on offset 197 — the last cell of a 200-cell pane — exactly where unicode-width puts it once the two wide glyphs are counted as 2 cells each. Two shell controls pin the terminal's own accounting rather than trusting the arithmetic: a 102-char line carrying two emoji occupies 104 cells in tmux's grid, and a line Python computes as 201 cells wraps at that point. The row's remaining half is now bounded rather than open-ended: the app-side F12 `d` buffer dump does not exist in this build (`handle_key_event` binds F12 to the debug-hub overlay only), so "the terminal renders it right" is proven while "ratatui's buffer agrees cell-for-cell" is a named residual with a concrete prerequisite.
- **iter-1391 — `/export` writes a real transcript from a channel (RT-179 item 3.4).** The TUI's twin is `/transcript`, which writes the same markdown to `~/.operant/transcripts/` and opens it in the external editor. A channel has no editor, so the reply names the file instead. It rides `run_session_scoped_command` for the same reason `/compress` does — the conversation being exported is THIS chat's loaded transcript, and reading the agent's cell without the scope would export whichever conversation happened to be loaded. The markdown comes from the TUI's OWN `export_as_markdown`, so both surfaces produce one format instead of two that drift. A `/export <path>` writes exactly where the operator points; a bare `/export` uses the TUI's timestamped default. The `Role::Tool` case is the interesting one: the TUI's message type has no Tool role, and dropping those messages would produce a transcript that silently omits what the tools did, so they are kept as System blocks naming the tool. Four tests, two mutation-proven: dropping the registry entry fails both command-level tests, and writing an empty file fails the runtime test that reads the file back.

### Added

- Nothing yet.

## [0.3.0] - 2026-10-26

1014 commits since v0.2.1 (2026-09-30), dominated by three waves: the jcode-parity composer upgrade (word completion, file discovery, scroll ladder, history-nav), the memory-wire deep integration (lifecycle hooks, wire-cost reduction, tool-surface consolidation, fused scoring), and the persistent-gateway daemon management suite (flock singleton, RAM budget, RSS visibility, queue-wait measurement) — plus the ponytail content-integration wave, the phantom-`process` parser fix, and the tooling-surface red-team execution.

### Added

- **iters 1335-1338 — persistent-gateway daemon management: the singleton, RAM budget, measurement, and the agent-per-seat gate.** Four findings of the 2026-10-26 persistent-gateway session/memory-management contrast audit (`docs/audit/2026-10-26-persistent-gateway-session-memory-management-jcode-contrast.md`), each closing a measured gap against the jcode reference: (1) **iter-1335 — the daemon flock.** `start_gateway` checked only an in-process static, never the pid file, so two `operant gateway run` processes started side by side — double-polling Telegram/Discord (fighting over getUpdates offsets), double-holding model credentials, double the RSS. An exclusive `flock(2)` on `$OPERANT_HOME/gateway.lock` is now taken at `start_gateway` entry and held for the daemon's life (dropped on exit, released by the kernel on crash, so a dead gateway can never wedge the next start); a second start is refused with a message naming `status`/`stop`. (2) **iter-1336 — daemon RAM management.** The gateway is long-lived with bursty per-turn allocations on the system allocator; glibc's one-arena-per-core default let freed-but-retained arena pages accumulate as RSS no live object explains. `allocator::configure()` (M_ARENA_MAX cap, first thing at daemon start — `MALLOC_ARENA_MAX` in the operator's environment still wins) plus a debounced `malloc_trim` at turn end (2 s min interval) keep the daemon's RSS bounded; vendored from jcode's allocator module with attribution, trimmed to the two functions the daemon needs. (3) **iter-1337 — `gateway status --memory`.** The daemon's RAM was invisible from every surface; `operant gateway status` now prints `Memory: RSS n.nn MB (peak n.nn MB)` from the daemon's `/proc/<pid>/status` VmRSS/VmHWM, with the same object in the `--json` arm (`memory: {rss_kb, hwm_kb, rss_mb, hwm_mb}` or null; an unreadable pid is `unknown`/null, never a fabricated 0). (4) **iter-1338 — the queue-wait measurement.** The shared one-agent design serializes every chat's turn behind `turn_lock`; whether that ever costs a user anything is now measured (events, total and worst wait, busiest chat) with a per-turn tracing line and an aggregate line every 50th queued turn — the ponytail gate the agent-per-seat upgrade path is explicitly NOT taken until worst_ms crosses into talk-time scale.
- **iter-1297 — memory lifecycle hooks: session-end, pre-compress, delegation, queue-prefetch (RT-191).** MemoryWireProvider now implements the four lifecycle hooks the agent loop already emits but the provider dropped on the floor (the audit measured the cost: only 4 recallable turn rows in the live bank because sync_turn was the only write path that fired): `on_session_end` folds the conversation's own prose (user/assistant pairs, HEAD-anchored 300/700-char bounds) into one `session-summary` document (Replace on re-fire, never duplicates); `on_pre_compress` appends a bounded gist per compression event and returns the pre-compression memory context the preflight ladder places (the trait default returned empty, so the ladder's live insertion always inserted nothing); `on_delegation` retains task + result-head per observation; `queue_prefetch` is a bounded 8-deep backlog the next prefetch drains. Store-contract correction proven RED-first: memory-wire refuses Append under a reused document_id, so append paths pair with `document_id: None` and rely on content-digest dedup.
- **iter-1301 — `memory_search` reports the real fused score (RT-193).** `MemoryWireProvider::recall_scored` runs `recall_with_weights` with FusionWeights::SHIPPED and returns the scored hits, so the fused RRF score finally survives the wire instead of dying in formatting; `MemorySearchTool` serializes it as `relevance` — the fabricated 1.0 is gone.
- **iter-1328 — context management: the SMOL model role is configurable, the ladder is visible, and the semantic stage is wired.** The 2026-10-25 context-management audit's G4/G5/G6 follow-ups, plus the omp `smol`-role parity the advisor/recap ports were missing. (1) **The summarizer model is no longer hardcoded** (G6): `[auxiliary_models.compression] model` now feeds `LlmCompressorConfig::summarizer_model` through one resolver (`llm_compressor_config`), replacing the `..Default::default()` spread that pinned `gpt-4o-mini` on every route — on a non-OpenAI endpoint enabling `context_compression` made every compaction fail and silently degrade. The slot existed in the setup wizard since it was written; nothing read it until now. `agent.context_compression_summary_max_chars` bounds the summary bill (u32::MAX default — TOML integers are i64, `usize::MAX` made the config file unwritable; the wizard's persist test caught it). (2) **Recap + advisor ride a cheap side-channel model** (omp model-roles parity): new `[recap.model]` slot (`attach_recap_engine` takes it; empty = the primary model, the historical behavior) and `[advisor]` keeps its existing slot — the advisor's hardcoded 120s side-call timeout is now `[advisor] timeout_secs` (its `Default` also fixed: a derived Default handed the engine a 0-second timeout when constructed in code). (3) **The preflight ladder is visible** (RT-182): it emits `CompactionStarted` when it fires and `CompactionCompleted` after the eviction — the same events the TUI already renders for the reactive path. (4) **The semantic compaction stage is live** (RT-183): `semantic_compaction_cutoff` runs as preflight rung 3 (between decay and the LLM summarize), armed by `agent.context_semantic_compaction` (default off) + the same `context_lcm_embedding_model` embedder knob that feeds lcm_vector_recall (`"local:hash"` = zero-dependency); every degrade path is the stage's own recency fallback. Parse test `context_management_knobs_parse_and_default` pins every new knob's TOML spelling and default.
- **iters 1306/1307/1311/1314 — ponytail as the fourth self-updating integration (content kind).** [DietrichGebert/ponytail](https://github.com/DietrichGebert/ponytail) — the "lazy senior dev" prompt/skill package — now installs and self-updates like memory-wire / sourcehound / AFT, but as the first CONTENT integration: upstream's GitHub releases carry no assets, so the tag tarball itself is the payload. `InstallKind::Content` on the shared binary_updater fetches the tag tarball, extracts only `skills/` (component-matched so nothing else smuggles in; symlinks/oversize refused; caps 512 KiB/file, 4 MiB total), runs the community-trust skills guard per extracted skill (Dangerous refuses the install and keeps the prior version; Caution installs with a warning), and lands under versioned `~/.operant/integrations/ponytail-vX.Y.Z/` dirs with keep=2 pruning — NEVER in the operator's `~/.operant/skills/`. Installed skills are exposed through `SkillManager` extra roots: invocable via `/skill ponytail-review`, listable in `operant skills list` (marked `[integration]`, `origin` field in JSON), present in the TUI skills overlay and typeahead, with the operator's own pool winning name collisions and delete refused for vendor-managed skills. `[agent.ponytail]` config (`enabled = false`, `default_level = "full"`, `inject = "on_invoke"` — all defaults OFF, the posture changes what the model is told every turn so it never silently enables): `enabled` gates the startup background update check; `inject = "always_on"` rides the frozen system prefix as a `<ponytail>` block (upstream's core ruleset + a level suffix line for lite/ultra) after the skills guidance, NEVER on Holon runs. The `/ponytail [lite|full|ultra|off]` TUI command flips the posture live (process-local override; config on disk untouched; refuses a level when the integration is not installed). `operant update --integrations` and `operant doctor` gained ponytail rows. A level or install change invalidates the provider prompt cache — documented consequence, same class as any config change.

### Fixed

- **iter-1344 — a refused `gateway run` hung silently in the signal wait instead of exiting.** Live-verification of the iter-1335 flock caught the follow-on defect unit tests could not see: a second `operant gateway run` printed nothing and blocked until timeout, because `cmd_run` unconditionally fell through to the SIGINT/SIGTERM wait after a refused start. `cmd_run` now reads the in-process runner registry after `start_gateway` and returns on refusal — the process answers the question instead of stalling.
- **iter-1280 — the phantom `process` storm: parser prose heuristic fabricated tool calls the model never made.** Four recurrences (10-20/10-23/10-24/10-25, all "test your tools" turns): the model streamed a markdown report table as its final answer — native tool_calls EMPTY, finish_reason `stop` — and the `ToolCallParser` plain-text heuristic (armed on ALL text since its nemotron-parity landing) turned every pipe-carrying prose slice into a fabricated `process` call with a nanosecond-stamp id. The 2026-10-25 specimen: 136 phantom calls quoting report slices, 136 `Unknown action` failures, the failure ladder halted the turn at 12 (`circuit_breaker`, tool_turns=212), and 136 corrupt tool rows persisted into the session context — then the halt wrapper told the operator a tool "kept failing" the model never called. Fix: the heuristic is now armed only INSIDE explicit `tool_call` blocks (the parser's existing `in_tool_call` flag), where content is an attempted invocation by construction; JSON-shaped recovery still runs everywhere and tagged nemotron-style calls still parse. Six regression tests including the streamed storm specimen (mutation-RED reproduces `['process','process','process','process']` re-arming the old behavior) and two tagged-parity pins.

- **iter-1279 — the history-nav fix: prompt history that works as history.** The operator's three reports against the composer, all root-caused against OMP (`../external-repos/oh-my-pi`) as the reference architecture: (1) **Up/Down got stranded in slash-completions.** Recalling a `/…` entry re-opened the typeahead popup (`update_suggestions` saw the leading slash through the shared refresh path), and the next Up/Down then fed the popup instead of stepping history. OMP auto-triggers autocomplete only on a TYPED trigger character in its character-insert path; programmatic recalls never pass through it. `history_up`/`history_down` now arm a one-shot `suppress_suggestions_once` the shared refresh consumes, and the Up/Down key arms check the browsing state (`history_pos.is_some()`) BEFORE any popup branch (OMP precedence: `#historyIndex > -1` first). (2) **The ladder was 70% commands.** 593 of 853 persisted history rows were `/help`-style commands crowding out real prompts; the operator's directive (commands must never appear in prompt history — stricter than OMP's secret-only `shouldSkipHistory` filter, and the directive wins) is now enforced at BOTH ends: `take_input` skips the ladder for slash commands, and `load_prompts_only()` filters legacy rows at seed time without rewriting anyone's file. The Ctrl+R search overlay still searches the FULL file (commands included — searching commands is useful) merged with the live in-session ladder. (3) **Ctrl+R Enter pasted the wrong entry.** `current_entry` indexed the caller's raw oldest-first non-deduped slice with the snapshot's newest-first deduped index — two different orderings, so Enter pasted whatever sat at that position; it now reads the overlay's own snapshot (the text each row displays). (4) **Alt+R re-submitted the previous prompt.** The empty-composer arm pulled `history.last()` unconditionally; OMP's retry is scoped to the last FAILED turn, and operant already had the `failed_input_recovery` slot for exactly that — the arm now refills from that slot only ("Nothing to retry — the last turn did not fail." otherwise). (5) **Ctrl+P discoverability.** Ctrl+P is the workspace ripgrep file search (real feature, catalogued in `/keys` since iter-429); its modal title now says "File search" instead of the anonymous "Search" (goldens regenerated). Every fix carries a mutation-proven regression test (suppress-arm, ladder guard, snapshot read, retry-slot — four mutations, four red-to-green). README documents the composer-history keys.

### Added

- **iters 1226/1233/1243/1246/1254+1261/1263+1267 — the 10-lane survey gap execution (TUI tooling, honesty fixes).** Six user-facing gaps from the red-team survey, each shipped with mutation-proven regression tests: (1) **iter-1226 — keyboard transcript scroll un-gated.** PgUp/PgDn were gated on a `FocusTarget::Transcript` state only a mouse click could set, so a keyboard session could not scroll at all. PgUp/PgDn now page unconditionally (step = the live viewport's transcript height minus two, not a hardcoded 10); Ctrl+Up/Down fine-scroll 3, Alt+Up/Down fast-scroll 20; Shift+Home/Shift+End jump to the transcript top/tail (unmodified Home/End stay composer caret moves). Live-verified in tmux against the real binary: keyboard-only sessions scroll and the status line announces "Top of transcript." / "Latest — following new messages.". (2) **iter-1233 — five dead tools purged.** `feishu_doc_read`, `feishu_drive`, `openrouter_query`, `xai_http_request`, `neutts_synthesize` deleted (1,197 lines): each duplicated a capability a live surface already serves, and none had credentials anywhere in this tree's config or `.env`. Pin test keeps them un-registered and un-advertised. (3) **iter-1243 — the todo card persists.** `TODO_STORE` was process-local, so every restart honestly reported 0/0. Now file-backed under the one-home resolver (`<home>/todos/`, hashed filenames — model-supplied ids are untrusted path material), atomic temp+rename writes, load-on-first-read, merge-into-persisted, explicit clear removes the file. The info-widget header renders "· N doing". Live-verified end-to-end: write → file → fresh-process read-back → clear-removes. (4) **iter-1246 — browser_dialog/accessibility_tree no longer demand BROWSER_CDP_URL.** Both now follow browser_cdp's design: explicit cdp_url/env first, else the managed shared session (sourcehound cloakctl.cdp auto-start/attach) over the persistent socket with page session ids. Live-verified: the dialog tool reaches cloakctl.cdp with no env var set. (5) **iter-1254 + iter-1261 — vision_analyze stops faking native vision.** The substring gate (`base_url.contains("openai")`) and its `_multimodal` envelope claimed "you can see it natively now" while nothing consumed it (its consumer died with the deleted operant-providers crate) — a model believing it would attach an image and hallucinate. The gate, envelope, module doc, tool description, and schema summary now all state the honest contract: the image never reaches the caller's context; the one real path is the auxiliary vision LLM (`auxiliary_models.vision` in config or `OPENROUTER_API_KEY`), and the result is its text description. (6) **iter-1263 + iter-1267 — /hotkeys tells the truth about the scroll ladder.** The catalogue still said "Scroll transcript up/down 10" (stale since iter-1226's viewport-derived step) and never listed Shift+Home/Shift+End; both fixed in iter-1263, pinned by a catalogue-vs-dispatch test that matches key AND modifier. iter-1267's validation pass then caught a dead chord the same test shape exposed: the Global table still advertised **Ctrl+G** as "Toggle scroll bookmark" after the bookmark moved to **Alt+G** (Ctrl+G became the external editor) — only the Prompt table had been corrected. Fixed in the Global table and the README, and the pin test now covers the g chords in both contexts, so a context-split drift cannot ride again.
- **iters 1251-1257 — the tooling-surface red-team execution (plan `docs/plans/2026-10-25-tooling-surface-red-team-process-args-parse-dead-end-tool-ro.md`).** The 2026-10-24 in-TUI tooling test ended in a 12-failure storm on `process` and a wall of `✗ process · 33 tok` rows. Root causes, all fixed: (1) the args-repair Pass 0 comment promised Python `strict=False` control-char acceptance but called the strict parser — raw newlines inside JSON string args (multi-line shell commands; answer-dumps) were always Unrepairable, 130 warnings in four days; the escape pass now exists (`repair_tool_call_arguments`). (2) The parse-failure refusal carried no diagnostic — it now names the serde error plus a 160-char args preview and offers the plain-text escape hatch. (3) A parallel batch drained before the fail-streak ladder could act — 32 of 64 doomed calls executed; the pre-flight now gates per call on the live streak plus same-batch refusals, answering the rest with guardrail skips (storm replay: exactly 8 refusals, 56 skips, nothing executed). (4) The failed-tool-row wall: consecutive identical failures fold into one ×N row and the failed header names the error (`invalid arguments: <reason>`). (5) The interactive TUI defaults to `<home>/logs/operant.log` (rotation-capped) instead of a discard sink — a default-config run logs nothing no more. (6) Doctor names the `[memory] provider = "builtin"`/`"disabled"` pins that leave memory_store/recall/search dead.
- **iter-1234 — word completion is ON by default.** `[tui] word_completion` now defaults to `"ngram"` instead of `"off"` (owner request: a cold-start install gets the full composer surface). The engine degrades to silence on a thin history — the τ=0.15 show gate offers nothing rather than junk — so defaulting on costs nothing fresh, and the predictor learns from the first submitted prompt. Set `word_completion = "off"` to opt out.

- **iter-1231 — prompt history rows carry `cwd` and `use_count`, deduped by content.** `history.jsonl` rows are now `{ts, text, cwd, use_count}` (old 2-field lines load unchanged, count 1). Appends normalize first (CRLF fold, per-line trailing-whitespace strip, outer trim — OMP's `normalizePrompt`) then dedupe BY CONTENT: a resubmission bumps `use_count` on the existing row instead of appending a byte-distinct duplicate, so terminal-padded copies of the same prompt count once. The `cwd` column records the submission's project directory — the corpus signal the word predictor (iter-1229) and the Ctrl+R history overlay both read.
- **iter-1230 — ghost-text word completion in the composer.** With `[tui] word_completion = "ngram"`, the prose word at the caret completes as DIM GHOST TEXT from an engine trained on your own submitted prompts (see iter-1229). TAB accepts with a trailing space, → accepts verbatim, typing through keeps the ghost (projection), diverging letters drop it and ask fresh. The ghost never appears while the suggestion popup is open, inside `@file:`/`@folder:` references, in slash-command input, inside backtick code spans, or mid-word. (The engine shipped defaulting to `"off"`; iter-1234 flipped the default to `"ngram"` — see below.)
- **iter-1229 — personal-history word prediction engine (`prompt_input/prediction/`).** The OMP `pi-predict` n-gram architecture, in-process: an interpolated absolute-discounting trigram over the prose words of submitted prompts (append-only overflow vocab ids with compaction — mid-learn id shifts corrupted counts and are the one bug the port caught), paste-weighted learning, `min 2 distinct prompts` hygiene, prompt-local + session caches, the prefix-aware posterior (ghost stability: typing through a shown word never reorders it), τ=0.15 show threshold. Snapshot persisted to `<home>/predict/ngram.snapshot`, atomic; corrupt snapshots wipe and re-ingest history. Measured on a real-history replay (783 prompts, 195 holdout): 6.46% keystrokes saved, 2.9 wrong-ghost words per 100 — a fair cold-start profile vs OMP's 33% at a 75× larger corpus.
- **iter-1218 — OMP-style file discovery for the `@` popup + async fuzzy tier.** `tui/file_discovery/`: a gitignore-aware recursive walk (the `ignore` crate) behind a shared scan cache (1 s TTL, 200 ms empty-recheck, 16-root cap), scored with OMP's tiers (exact 100 / prefix 80 / substring 60 / subsequence 40−5·gaps on the filename, weak path tier, +10 directory bonus, shallower-then-lexicographic tiebreak) under a 1.5 s deadline returning partial results. The `@` popup now finds NESTED files: the immediate directory listing renders synchronously while the recursive fuzzy walk runs on the blocking pool (single-flight, newest-queued, generation-checked, stale-list narrowed so every keystroke shows something coherent).

### Changed

- **iter-1227 — the `@` popup inserts the reference grammar.** Accepted files insert `@file:<path>` (trailing space, the accept path appends it), directories insert `@folder:<path>/` and CHAIN — the popup stays open drilling into the directory (no space). Paths with spaces quote (`@file:"a b"`; a directory quote stays open until a file inside it closes the reference). Trigger tokens are `@`, `@file:`, `@folder:`. This closes the coherence gap where the popup's bare `@path` tokens were never expanded by `context_references` — accepted references now demonstrably inline file content into the turn (pinned by an end-to-end round-trip test through `preprocess_context_references`).
- **iter-1217 — OMP word classes for the composer's word-wise operations.** Word boundaries now distinguish word / delimiter-run / CJK classes with joiners: `don't` and `well-known` move, kill, and delete as ONE word (Ctrl+W, Alt+Backspace, Alt+B/F, Ctrl+Left/Right all share the model), punctuation runs (`->`, `::`) are single stops, CJK runs move per segment. Home/End are line-scoped on multi-line drafts (Super+Left/Right remain the buffer jumps).

### Fixed

- **iter-1242 — red-team pass over the composer upgrade and the transcript scroll: four root causes fixed.** Found by driving the operator's live reports against the headless sim and the real renderer. (1) **Ctrl+Backspace** reached the app as `Char('h')+CONTROL` — terminals send byte `0x08` and crossterm maps `0x01..=0x1A` to Ctrl+letter, so the `Backspace+CONTROL` arm never fired on the terminals people actually use and the chord was a silent no-op (measured on the sim). `Char('h')+CONTROL` now shares the word-delete arm; a real `Backspace+CONTROL` (kitty-protocol terminals) keeps working. (2) **The `@` popup could never show its sync tier**: it consumed `file_autocomplete_limit`, whose serde default — and the value every pre-existing `settings.json` carries — is **0**, so the immediate directory listing broke on the limit before emitting one entry (the settings screen even displayed "15" while the stored value was 0). 0 now coerces to the 15 the screen always advertised; the async fuzzy tier was never limited and is unchanged. (3) **Scrolling moved in the wrong direction**: the renderer consumes `scroll_offset` as the viewport's TOP ROW (`materialize_line_slice(scroll, …)` — 0 is the oldest line, the frame max the tail, the same row a tail-following frame paints), but every keyboard and wheel handler was written against the opposite convention (0 = tail, `+=` = up), so while auto-follow painted the tail the state sat at 0 — the first wheel tick or PageUp teleported the view to the TOP of the transcript and later presses walked back DOWN ("scroll up jumps to the top, then scrolling up scrolls down"). One convention now: `scroll_up_rows`/`scroll_down_rows`/`scroll_to_top`/`scroll_to_tail` implement the directions (up = toward row 0, down = toward the tail, re-following on arrival), the reconcile syncs the state to the tail while following so a first scroll is relative to where the reader actually is, resize adoption writes the resolved top row directly, and the inverted-convention growth-correction machinery (whose own header claimed "0 IS the tail" against the renderer's evidence) is deleted — under the top-row convention appends cannot move a parked row. Copy-mode edge autoscroll flipped with the rest; Shift+Home/End land on the true top/tail (verified live in tmux). (4) **The `-- INSERT --` badge** proved to be the persisted `vim_enabled: true` in settings.json plus a composer that can never leave Insert (no live path routes Esc into the vim machinery), so the badge is permanent noise for a mode that cannot function — the operator's setting was flipped off with `operant tui vim off`; the dead vim surface is recorded, not "fixed" in code.
- **iter-1221 — "every N<h|d|w>" cron schedules were calendar steps, not durations.** `normalize_schedule` expanded interval phrases into cron day-of-month fields ("every 48h" → `0 0 0 */2 * * *`), a calendar rule that fired Jan 31 then Feb 1 — one day apart — and gave "every 2w" a 3-day gap across February; "every 25h" was rejected outright and "every 30d" silently became monthly-on-the-1st. The agent-side `cron_tool::compute_first_run` always had the correct duration semantics, so the two surfaces disagreed. Interval phrases are now their own canonical form: persisted raw, never rewritten, `next = now + period` everywhere (`schedule::interval_seconds`, `next_run_from_schedule`, and the scheduler's `compute_next_run` no longer persists a cron expansion for them). Cron expressions keep their calendar semantics unchanged, and jobs already stored with the old expanded cron forms keep firing on that rule (re-create the job to switch to the duration). Regression tests pin the audit's concrete cases: `every 48h` from Jan 31 23:00 UTC lands Feb 2 23:00 (was Feb 1 00:00), `every 2w` from Jan 29 lands Feb 12 (was Feb 1). Audit finding 4 of `docs/audit/2026-10-24-ponytail-whole-repo-quality-audit.md`.
- **iter-1220 — Stripe webhook verification could never succeed.** `verify_request_signature` folded `stripe-signature` into the generic branch, comparing the *whole* `t=<ts>,v1=<hex>` header against a plain `HMAC-SHA256(secret, body)` — Stripe signs `HMAC-SHA256(secret, "{ts}.{body}")`, so no genuine Stripe event could ever match and every one was 401'd (Stripe retries, then disables the endpoint). Stripe now has its own branch: the header is parsed into `t=` and one-or-more `v1=` entries (multiple `v1` are Stripe's signing-secret rotation shape — any match verifies), the HMAC covers `"{ts}.{body}"`, and the same 5-minute replay window Slack uses rejects stale timestamps. Five tests pin the contract (genuine, rotating, wrong-secret, stale, malformed-header); the Slack and GitHub paths are unchanged. Audit finding 2 of `docs/audit/2026-10-24-ponytail-whole-repo-quality-audit.md`.
- **iter-1219 — the webhook adapter listened on every interface by default (`0.0.0.0:8080`), while the signing secret is optional.** With no `[gateway] webhooks_secret` the whole HMAC block is skipped, so "enable webhooks" without extra settings meant an unauthenticated command channel pointed at the agent on all interfaces — the startup warning named the risk (iter-1009) but shipped the risky default. The default is now `127.0.0.1:8080` via one shared constant (`WebhookAdapter::DEFAULT_LISTEN_ADDR`): the adapter constructor and the gateway factory's fallback literal (which had its own second copy of `0.0.0.0:8080`) both read it, the setup wizard's prompt shows the loopback default and names what `0.0.0.0` means, and `operant.example.toml` says the same. Reaching beyond the machine stays a one-line opt-in: set `webhooks_addr = "0.0.0.0:8080"` — pair it with `webhooks_secret`. Found by the 2026-10-24 ponytail whole-repo audit (`docs/audit/2026-10-24-ponytail-whole-repo-quality-audit.md`); the regression test `default_listen_addr_is_loopback` pins the constant and the constructor.
- **iter-1171 — the clippy warning gate's allowlist is EMPTY: `seen=0 allowlist=0 new=0 stale=0`.** The gate reported `seen=1 allowlist=24 new=2 stale=23` right after iter-1169, and both NEW warnings were this sweep's own residues: a `clippy::duplicated_attributes` in `swarm_gallery.rs` from the **first** mutation test (an `#[allow(dead_code)]` the restore did not revert, leaving two suppressions on one item), and an `unused_imports PathBuf` in `operant-infra/session_sqlite.rs` that the iter-1163 `db_path` deletion orphaned while nothing re-checked that crate. Both fixed, the allowlist (keyed by `crate|lint|file`) pruned from 24 to 0 because the files no longer warn at all. **The mutation-test lesson, recorded because it cost a reopen:** a mutation on an item that ALREADY carries a suppression matches an existing baseline key and is correctly *not* reported as new — so that mutation proves nothing and leaves residue behind. The valid mutation is one on an unsuppressed item. Verified: `cargo clippy -p operant-cli --bin operant --all-targets` emits zero warnings after forcing a rebuild of every touched file, production profile 0, operant-cli 1638, operant-infra 64, suppression gate 119/119, all pass.
- **iter-1169 — `cargo clippy -p operant-cli --bin operant --all-targets` emits zero warnings; it measured 54.** Every one of the 54 was `dead_code` in the TEST profile, with the production profile clean at 0 throughout (re-checked after each file, never assumed). Each was one of: a wire-format field the tests parse but no live row reads; render state the live panel never enters; a picker action or result vocabulary no live picker returns; a config-shim field kept so the shape matches upstream. Each is now `#[cfg_attr(test, allow(dead_code, reason = "..."))]` — the TEST-profile arm, because an `#[expect]` would be **unfulfilled** in production where the lint does not fire (that measurement is what chose the arm). The idiom is the one `operant_model/adapter.rs` already documents per rust-best-practices ch.2: name the dead variants in the module head, keep the explicit match arms, so the first live construction is a visible decision point rather than a silent behaviour change. RT-168's `CopyTargetKind::label()` / `copied_notice()` now carry the reason *being* RT-168, so the row that records the uncollapsed duplication has the suppressions pointing at it. Verified zero after forcing a rebuild of every touched file; operant-cli stays 1638 passed / 0 failed.
- **iter-1168 — the suppression gate governed 30 of 80 sites, because it could not see the other 50.** `scripts/audit-lint-suppressions.py` (iter-1164) matched only the bare `#[allow(...)]` / `#[expect(...)]` spelling, so every `#[cfg_attr(not(test), allow(...))]` and every rustfmt-split multi-line attribute was invisible to it — including ones carrying their own `reason`. A gate that cannot see a suppression cannot govern it: the same defect class RG-013 records for the doc gate, recreated in the tool meant to close it. Four defects, all measured: `RE_ATTR` could not align the cfg_attr form (in `#[cfg_attr(test, allow(...))]` the `allow` sits INSIDE cfg_attr's parens, not after them, so the optional-prefix pattern returned `False` for every cfg_attr line); a per-line scan cannot see an attribute rustfmt split over four lines (the scan now joins each unclosed `#[...]` through its matching paren, keeping original line numbers); `item_after()` walked from the attribute's first line and read the cfg predicate `test,` as the item name; and `item_after` was called with a 1-based line number as a 0-based index. Now 119 suppressions across 61 files, 119 baseline entries, 0 unexplained. Mutation-checked: a suppression added to an unsuppressed item makes the gate name the file and line and exit 1 — and the first attempt at that mutation wrongly targeted an already-suppressed item, which the gate correctly did not report as new.
- **iter-1166 — the doc gate went RED the day after it was wired in, and the cause was prose.** `crates/operant-cli/src/tui/operant_ui/ui_changelog.rs` documented the embedded changelog record format as the literal string `hash<RS>tag<RS>timestamp<RS>subject`. The `<RS>` is a real ASCII RS (0x1E) field separator — `parse_changelog_from_impl` splits with `splitn(4, '\x1e')` — but rustdoc reads it as an unclosed HTML tag under `-D rustdoc::invalid-html-tags`, so `scripts/check.sh doc` exited 1 with three errors plus `could not document operant-cli`. The gate iter-1158 turned green was red again within one iteration of being wired in. The doc now names the characters ("ASCII RS (0x1E)" / "ASCII US (0x1F)") and puts the format in a code span so rustdoc reads it as source. No parser behaviour changed: the separator was always `\x1e`, only the description was wrong about how to write it. Gate exit 1 → exit 0; operant-cli 1638 passed / 0 failed.
- **iter-1164 — the lint-suppression surface is a gate now, and the number in the old plan was wrong by an order of magnitude.** `docs/RUST_BEST_PRACTICES_PLAN.md:25` still reports **242** `#[allow(dead_code)]`/`#[allow(unused_*)]` suppressions from a 2026-08-02 measurement (sourced from `docs/DEAD_CODE_GAP_ANALYSIS.md`, whose own breakdown says 197 + 45). The live figure is **33** — most of it retired with the config-derive machinery, the parallel config aggregate, the retired provider surface and the three deleted crates. A prose number cannot stay true across that, so the claim is now `scripts/audit-lint-suppressions.py`: a reason-carrying baseline, `--check` that fails only on a suppression the baseline does not explain, and a `CONVERSION CANDIDATES` list for the sites where an `#[allow]` could be an `#[expect]` and is not. It is a step of `scripts/self-test.sh` now. Mutation-checked: adding an unexplained `#[allow(dead_code)]` to `operant-harness/src/row.rs` makes the gate name the file and line and exit 1.
- **iter-1160 — five `#[cfg(test)]` wrappers that forwarded to the live implementation and nothing called, plus one accessor whose documentation contradicted its behaviour.** An autonomous defect sweep measured `cargo clippy -p operant-cli --bin operant --all-targets` at 69 warnings, all `dead_code`, all jcode-port residue, with the PRODUCTION profile clean at 0. Five of them were test-only wrappers whose only job was forwarding to the `_impl`/`by_index` sibling that is itself the live code (`parse_batch_sub_outputs`, `parse_changelog_from`, `group_changelog_entries`, `calculate_input_lines`, `debug_probe_pinned_diagram_with_font`); each was confirmed zero-caller across `crates/` before deletion, and the `_impl` functions stay because `parse_changelog()` and `get_grouped_changelog()` call them. The sixth was `config_shim::test_guard_lock()`, documented in its own comment as "ONE lock, shared by the guard and by the tests that inspect it — a per-caller static would not serialise anything", which returned a **different** static than the one `reasoning_override_test_guard()` actually takes, and had no caller anywhere — an accessor that defeats its stated contract. `todo.rs`'s `semantic_state!` macro now carries `#[allow(dead_code)]` with the reason, because it generates one uniform schema for eleven enums and deleting an accessor per enum would break the schema. 69 → 54 warnings; the operant-cli suite stays 1638 passed / 0 failed.
- **iter-1161 — two struct fields written and read by nothing, and the duplication the second removal exposed.** `SystemProfile` stored five detection inputs (`load_avg_1m`, `cpu_count`, `available_memory_mb`, `total_memory_mb`, `is_ssh`) that `compute_tier` consumes once during detection and nothing reads afterwards; the struct now carries exactly its consumed surface, with the detectors named so a future consumer knows they are cheap to re-detect rather than having to restore five dead fields. Field order is load-bearing and now commented: `fragile_glyph_cache` borrows `terminal` before the move, which the compiler caught as E0382 on the first attempt. `VisibleCopyTarget` built `kind_label` and `copied_notice` from `target.kind` per target per frame and read them nowhere. Removing that pair exposed **RT-168**: there are TWO `CopyTargetKind` enums — `operant_markdown/markdown_types.rs` (the one `prepared.rs`, `operant_ui/mod.rs` and `render/selection.rs` import) and `copy_targets.rs`, whose near-identical `label()`/`copied_notice()` methods are now referenced only by their own tests. The surviving callers and the surviving tests sit on opposite sides of the duplication. Recorded as a ledger row and as `docs/audit/2026-10-21-copytargetkind-duplication-and-the-54-dead-code-warnings-fro.md`, and deliberately NOT fixed: `copy_targets.rs` also defines `CopyTarget` and the extraction helpers its tests drive, the `Math { display: bool }` vs `Math` difference is semantic, and two modules cite it in `[port-decision]` comments the fix's commit must move with the code.
- **iter-1159 — the release binary the gate built could not start: the build environment had two copies and they disagreed.** `scripts/check.sh` re-implemented the build environment instead of sourcing `scripts/dev-env.sh`, and its copy had dropped the ONNX Runtime link flags — including the `-Wl,-rpath` that makes the shipped binary find `libonnxruntime.so.1`. A binary built through `check.sh` therefore carried no RUNPATH and exited 127 in any shell that had not sourced `dev-env.sh` — including the self-test, the TUI corpus runner and the operator's shell. The self-test reported 10 failed steps and 70 corpus "drifts" that were all the same string: "error while loading shared libraries: libonnxruntime.so.1". The corpus drifts looked exactly like a rendering regression and were not one. The environment now has one source (`check.sh` sources `dev-env.sh`; 85 lines of hand-rolled copy deleted, the two settings only `check.sh` had moved into `dev-env.sh`), and the rebuilt binary carries `RUNPATH [$HOME/.local/lib/operant]` and runs under `env -i`.
- **iter-1158 — the rustdoc gate had been RED with ~70 broken intra-doc links, and nothing ran it.** The repo's doc gate is `./scripts/check.sh doc` (the exact command the dead CI job ran); `scripts/self-test.sh` never had the step, so the gate drifted red unseen. It was red: unresolved links, unclosed HTML tags and links-to-private-items across five crates. Four classes, all fixed — links to items that were renamed or removed (repointed where the real target exists, de-linked where nothing does), links that could not resolve from their scope (`crate::provider::ProviderRegistry`, `operant_api::ToolCall::extra_content`, `App::push_notice`, `MessageBoundary::msg_hash`, and the six `session::autocompact` constants, whose fully-qualified path resolves where the bare name does not), markdown rustdoc reads as a link or an HTML tag (the permission dialog's `[y]`/`[Y]`/`[p]`/`[n]`/`[g]` keys, the `[holon]`/`[bylaws]`/`[display]` config sections, `<digits><unit>`, `MEDIA:<path>`, `Vec<bool>`, `[label](target)`), and links from a public doc to a private item. The gate command gained `--document-private-items` — without it every link to a private item reads as unresolved, and the gate cannot tell a private link from a broken one. The gate is a step of `scripts/self-test.sh` now.
- **iter-1157 — AGENTS.md's commands pointed at a directory that does not exist on this machine, and it named a git dependency that is deliberately held out.** The protocol file carried **15** absolute paths under `/home/z/my-project/…`; `/home/z` does not exist here, so `cd`, `source scripts/dev-env.sh` and every `./scripts/check.sh` line in the file failed on the machine that reads it. The build-environment block also spelled out a hand-copied `LIBCLANG_PATH`/`ORT_LIB_LOCATION` snapshot that `scripts/dev-env.sh` already derives from `LOCAL_DIR`. "Path Dependencies" said "two sibling-project git dependencies" and named `sourcehound_mcp` — which is **commented out** in `operant-core/Cargo.toml` with the reason, while the design-preferences section of the same file already said the browser is the `sourcehound` binary over MCP stdio. The tree's feature list for `operant-cli` named four features the crate does not declare. Every path is now repo-relative, the env block uses `$LOCAL_DIR`, the dependency section names ONE git dependency and explains the sourcehound hold-out, and `scripts/check-doc-claims.sh` gained two gates: the tree's `# Features:` list against each crate's declared `[features]`, and the prose's "`X` cargo feature" mentions. Both mutation-checked.
- **iter-1156 — AGENTS.md advertised 22 LifeOS tools, a `lifeos` cargo feature and a `NOTION_API_TOKEN` requirement; none of them exists in this tree.** A whole-workspace search for `lifeos` returns exactly one Rust file — `config.rs` — and its content is a note that the key was REMOVED plus the test that locks its absence. No `Cargo.toml` declares the feature; the tool module is gone; the CHANGELOG said so in the release that removed the key. The claim appeared four times, twice under "Unique Features (operant has, hermes doesn't)". The same audit found the `--all-features` table naming three crates deleted in iter-892 (and a paragraph instructing the reader to wire one of them). AGENTS.md now says LifeOS is NOT INTEGRATED, the deleted crates are marked, and `scripts/check-doc-claims.sh` gained a gate: every `` `name` cargo feature `` AGENTS.md claims must be declared by a crate (mutation-checked). The `--all-features` claim itself was re-verified — `cargo check --workspace --all-features` finishes with 0 errors and 0 warnings.
- **iter-1154 — the defect ledger's index said eleven sections were OPEN and nine of them hold no open row; the live gap inventory describes stores that were deleted.** A script that counts checkboxes per section finds nine `## OPEN — …` sections with zero open rows (110 closed rows wearing an OPEN heading), a preamble that said "None is fixed" with all six of its rows checked, and a malformed `- []` on RT-135 that hid it from any count. The index now names all six open items (RT-135, RT-153, RT-154, S2, D-5b, D-6) with their state, and the nine closed sections are titled CLOSED. `docs/REMAINING-GAPS.md` — a dated verification against iter-528 whose subject (the organ stores) retired to Holon under the owner order of 2026-10-15 — keeps its dated claims and carries a dated amendment pointing at what retired, at the live org surface (`operant org budget`, `operant org holon`), and at the two inventories that are current.
- **iter-1153 — the config-surface audit's `--check` mode could never pass, and the ledger promised the opposite.** `scripts/audit-config-surface.py` prints every settings field that nothing outside the loader reads; its `--check` mode exited non-zero whenever ANY candidate existed, and the candidates are the settings RT-136 accounted for and warned about — so the check was permanently red. The ledger said "`--check` exits non-zero when a NEW candidate appears", but the tool had no baseline and could not tell a new candidate from an accounted-for one. The count had moved too: the ledger says 24, the script says 27, because iter-1140's config retirement deleted the dormant schema files that had been the last mention of three live settings (`[memory] audit_enabled`, `[memory] response_cache_enabled`, `[tools.web] tavily_api_key`) — all three accounted for, none re-counted. The script now carries an `ACCOUNTED_FOR` baseline (each known candidate with the reason it is still there), `--check` fails only on a candidate outside it, and it is a step of `scripts/self-test.sh`. Mutation-checked: a new unread setting makes it print "27 accounted for, 1 new" and exit 1.
- **iter-1152 — the files the setup wizard and the OAuth store write were world-readable for the instant between the write and the chmod.** `operant_core::fs_secrets::write_secret_file` — the writer for the OAuth token store and for the setup wizard's config TOML (which holds the API key the operator has just typed) — opened the file with the umask default (0644), wrote the secret INTO it, and only then chmod'ed it to 0600. That is the class R40-1 closed in 2026-09-27, and the repo already carries the fixed pattern in two other places (`operant-config`'s identically named writer and `operant-cli`'s `.env` writer) — three implementations of one job, two fixed, this one missed. `config.rs`'s `persist_config_key` (the `operant config set` path) had the same window in its temp file, and that config can carry `client.api_key`. Both now create the file OWNER-ONLY in the same `open(2)` that creates it, and `persist_config_key` relaxes the temp file to the original's mode only after the write. The new test asserts the mode is already 0600 with ZERO bytes written; mutation-checked (dropping the `.mode(0o600)` makes it fail with "expected 0o600, got 644").
- **iter-1151 — `operant org`'s module documentation described a command family that was deleted a week earlier.** The file still carried the pre-G+1 org surface command by command (`org employee create|update|retire`, `org notice post|ack|pin|unpin`, `org worklog append|list`, `org grant give|revoke`, `org sync`, `org import`, `org check`, `org list`), a table of where each one's `--reason` is persisted, and a "Storage" section naming `EmployeeDb`/`NoticeBoard`/`WorklogDb` as stores operant owns. `OrgSubcommand` has two variants — `Budget` and `Holon` — the three store types are gone from the workspace, and the intra-doc links (`operator_entry`, `cmd_import`, `OrgEmployeeAction::Update`) point at items that are not in the file. The doc now describes the surface that exists, says plainly what retired to Holon and why, and keeps the `--reason` mandate with the table of where the reason actually goes today. The sibling test (`crates/operant-cli/tests/org_reason.rs`) already stated the truth in its own header; the module doc had not caught up.
- **iter-1150 — the README advertised three retired org mechanisms, the crate count was wrong in two documents, and the local gate had been red in its tail for five days.** The README's *Org governance* row promised "a TTL'd grant ledger", a "durable escalation queue" and "a senior's `/approve` mints a grant" — the grant ledger, the escalation queue, the employee registry, the hierarchy edges and the cast retired to Holon under the owner order of 2026-10-15, `SeatApprover::mint` does not exist, and the worklog STORE retired too, so operant writes no worklog row at all. `AGENTS.md` and the README also said "12 crates" after `operant-macros` was deleted with the retired config derive (the workspace has 11) — and `scripts/check-doc-claims.sh`, the repo's own gate for exactly this, had been reporting "DRIFT workspace members: doc says 12, code has 11" since iter-1141, because `scripts/self-test.sh` never ran it. The README row now says what operant does, the count is 11 in both documents, and the doc-claims check is a step of the local gate. Separately, the gate's own tail was broken: the corpus step's `cd` ran in the script's CURRENT shell (through `eval`), so it left the working directory inside the scenarios directory and every later step's `./target/release/operant` failed with "No such file or directory" — the seven CLI smoke steps had failed on every run since the corpus step was added at iter-997. The corpus step now runs in a subshell and the CLI steps name the binary absolutely.
- **iter-1149 — the config TEMPLATE advertised three examples the loader REJECTS, and a gate keeps it that way.** `operant.example.toml` is the file a new operator copies, and its COMMENTED blocks are the examples they uncomment — but `config.rs`'s `example_toml_parses` reads the live (uncommented) template only, so none of them was ever checked. Three would have broken the whole config (the settings structs are `deny_unknown_fields`, so one unknown key rejects the file): `fallbacks = [ { model = "gpt-4o-mini" }, … ]` (the live field is `agent.fallback_models`, a list of strings — RT-156 fixed exactly this block in the README and the template kept the old shape), `[agent] max_tool_result_chars` (the knob never existed in the live config: it lived in the retired schema aggregate, and the trim plus the artifact offload it drove died with the `operant-runtime` crate in iter-892 — the live guard is `agent.max_tool_result_share`), and a whole `[channels.telegram]`/`[channels.discord]` section for the `operant-channels` crate deleted in the same purge. A fourth claim in the same file was false and no loader could catch it: "(The run consults the grant ledger every tick regardless.)" — there is no grant ledger. The three are gone, the `max_tool_result_share` block now says the withheld part is not stored anywhere the agent can read it back, and the new test (`crates/operant-core/tests/example_template_loads.rs`) uncomments EVERY commented statement — alone and inside its enclosing table — and hands it to the real loader; it is mutation-checked against both `fallbacks` and `[channels.telegram]`.
- **iter-1146 — the README's config example now loads, and a gate keeps it that way.** The README's copy-paste config block carried `fallbacks = [ { model = "gpt-4o-mini" } ]` where the live field is `fallback_models` (a list of strings), and `grant_ttl_days = 7`, a key retired with the grant ledger. The settings structs are `deny_unknown_fields`, so the loader rejected the WHOLE file — every operator who copied the README's example got a config that would not load. The block is corrected, the retired key and the ignored `[tools.web] preferred_provider` are gone, the CI badge is removed (the Actions quota is exhausted, so no workflow runs on a push — a badge that can never go green is a false signal) and the README now says the local loop is the gate. A new test (`crates/operant-core/tests/readme_examples_load.rs`) parses every ```toml block in the README through the REAL loader and asserts no retired key is advertised; both assertions are mutation-checked.
- **iter-1141 — the `Configurable` derive machinery is gone and the parallel config surface is fully retired.** The derive existed for one reason: the retired config aggregate's property machinery (`secret_fields`, `prop_fields`, `integration_descriptor`). The live types derived it only for the `#[secret]` and `#[prefix]` attributes, and nothing called what it generated — so the derive is dropped from the six live types, the attributes go with it, and the `operant-macros` crate (1.3K lines) is deleted from the workspace along with the crate-level `helpers.rs` it needed and the `config`/`security` shim modules the macro's generated paths resolved through. `traits.rs` keeps only the two items that outlived it. RT-141 is CLOSED: the binary now carries ONE config system. The residue the retirement exposed is recorded as RT-154 — the `#[secret]` marker was the only thing that ever claimed the config's provider keys were encrypted, and nothing in the live path honoured it; the field's documentation now says what is true (plaintext in a 0600 file, with the ENCRYPTED store being the provider auth-profile file).
- **iter-1140 — the dormant config SECTION structs are gone: `operant-config` is 7.1K lines, down from ~31K.** The 17 wholly-dormant schema files are deleted (`agent_cfg`, `backup`, `channels_cfg`, `delegate`, `gateway`, `hardware`, `mcp`, `ops_cfg`, `runners`, `scheduler`, `security_cfg`, `skills`, `sop`, `stt`, `tts`, `tunnels`, `web_tools`), the six files that carry a live type are trimmed to it, and the crate's own dormant modules go with them (`scattered_types`, `comment_writer`, `domain_matcher`, `pairing`, `typed_value`, `api_error`, `validation_warnings`, `workspace`, `cost`, `platform`, `migration`). The compiler was the oracle: every item it reported as unreachable after the deletions was removed, and the items the live types still reference — the `default_*` helpers, the `HasPropKind` impls, the `Configurable` derive and its support traits — were kept. What remains in the crate is the live surface: the 12 types the production code names plus the five proxy client builders. The audit script now reports 10 schema structs and 1 flagged (a false positive: `EmbeddingRouteConfig` is carried by the live `ProvidersConfig`).
- **iter-1139 — the TUI's post-scroll ghost row is CLEARED by a live terminal check.** The `S1` row had been open since iter-973 because it needs a real terminal: the ghost it describes can only appear in the frame that comes BACK after a scroll. A live 200×50 tmux session with a transcript taller than the viewport, scrolled up and back, produces a frame byte-identical to the pre-scroll frame in every row except the live `⏱ N tok/s` counter — no fragment of the scrolled-past content, no half-drawn border. The same session also clears the AMBIGUOUS half of `S2`: every ambiguous-width glyph the TUI uses (`● ✗ ✓ › ▸ ⚑ ⏱ ⇧ … ·` and the whole box set) lands in exactly one cell in a 200-column pane, with the tool rows' labels and the panel borders aligned. The WIDE (emoji) half of `S2` is still open — the emoji rendered without a visible artifact, but the app's own cell buffer could not be dumped for a cell-for-cell comparison.
- **iter-1138 — the second config surface is GONE: the aggregate, its loader, its migration and its tests are deleted.** `operant_config::schema::Config` was an aggregate of ~30 sections with its own `config.toml` resolution, its own loader (`Config::load_or_init`) and its own v0→v1 migration — none of it ever loaded in production (only the crate's own tests did). About 9.7K lines go with it: the aggregate, `schema/config_impl.rs` (1.9K), the `V1Compat` migration and the 7.1K-line test file. The two production-typed consumers of it (`operant-providers::provider_runtime_options_from_config` and `operant-providers::auth::{from_config, state_dir_from_config}`) were called only from their own tests and went with it. The guard test is now stronger: it fails if the aggregate is USED anywhere in production code, and a second test fails if the type is DEFINED again. What remains in `operant-config/src/schema/` are the types the live config composes; the dormant section structs are the rest of RT-141 and stay open with the plan attached. Recorded along the way: the `operant-providers` crate is a second provider surface that production never calls (RT-153) — the live provider path is `operant-core/src/client.rs`.
- **iter-1137 — `operant profile use <name>` now actually switches the profile, and the operant home has ONE resolver.** The command wrote a sticky `<root>/active_profile` file and NOTHING read it: a whole-workspace search found no runtime path, no config loader and no store that consulted it, and a live probe (a sandboxed `HOME`, a profile with a distinctive `model` in its `operant.toml`) showed the next run still on the default config and the default database. Underneath that, the home itself was resolved nine different ways, and only one of them honoured `HERMES_HOME`: the DATABASE (`~/.operant/database.db`, so profiles shared the default profile's sessions, cron jobs, budgets and tool state), the TUI's `settings.json` and `auth.json`, the `.env` that `/connect` writes its endpoint pins into (the runtime read a different file back), the OAuth refresher's `auth.json`, the provider account index, the MCP OAuth token directory, the approval allowlist, the agents directory, the caches, the checkpoints, the transcripts, the stats, the history and the logs — plus two more spellings of the home that were never documented (`OPERANT_HOME`). All of them now resolve through `operant_core::platform::operant_home()`, which is `HERMES_HOME` (explicit, wins) → the sticky active profile → `~/.operant`. The sticky file's name is validated with the profile-id rule before it is allowed to steer a path, so a tampered `active_profile` is ignored rather than obeyed. A new guard (`crates/operant-core/tests/one_home_resolver.rs`) scans every non-test file in the workspace and fails when one builds an operant-home path from the OS home, so the second spelling cannot come back.
- **iter-1136 — the `accent_role_reaches_the_frame` flake is gone.** The palette is a process-global, and `App::new` writes it. One test in the TUI's render module built an `App` without taking the palette lock the theme-asserting tests hold, so it reset the palette between their `set_active_theme` and the read of the role palette their assertion compares against — the frame was painted under one theme and compared against another. The test now takes the lock, like its two twin helpers already do. The repo's own gate (`scripts/self-test.sh`) inherited this flake at roughly 1 run in 4.
- **iter-1135 — `operant tui keybindings` lines its columns up.** The action was rendered with a
  Debug format spec applied straight to the enum, and a spec on a tuple variant pads the FIELD
  rather than the variant: `KeyAction::Custom(1)` came out as `Custom(` + a 34-wide number + `)`,
  and a unit variant ignored the width entirely. The action is now rendered to a string first and
  padded after. Caught by the live probe of the deployed binary, not by the tests.
- **iter-1134 — the CLI test suite no longer overwrites the operator's real `~/.operant/gateway.pid`.** The gateway start/stop test isolated its home with a THREAD-LOCAL override, but the stores under the home read the ENVIRONMENT variable and never consulted it — so `start_gateway` wrote the pid file into the operator's real `~/.operant` and the stop deleted it. The test now isolates through the environment (process-global, so every thread sees it) and proves the isolation before writing anything. The broken seam is gone with it: the thread-local override and the second home resolver are deleted, so there is one resolver for the home.
- **iter-1133 — `operant tui keybindings` no longer writes a file nothing reads.** The command created
  `~/.operant/keybindings.json` and opened it in `$EDITOR`; the loaders were deleted in the 2026-10-14
  batch (plan 10) because they were no-op stubs, so editing the file changed nothing. It now prints the
  bindings that are in effect — the same catalogue `/keys` and `/hotkeys` read — and says plainly that
  custom overrides are not implemented, because the binding registry is the catalogue and not the
  dispatcher.
- **iter-1132 — the test suite stopped reaching GitHub, and the proxy guard it broke is green again.** `client::tests::env_proxy_is_ignored_by_the_provider_client` failed in the full `operant-core` suite and passed in isolation, on a clean `origin/main`. The connection the guard counted was not provider traffic: it was `CONNECT api.github.com:443`, made by the BACKGROUND auto-update check, which fires on the first use of the aft/sourcehound integration, shells out to `gh`, and lands in whatever test is running at the time. The background check now does not run in a test build, and `OPERANT_NO_AUTO_UPDATE=1` turns it off in any binary. The explicit paths (the doctor's integration check, `operant update --integrations`) are untouched.
- **iter-1132 — `operant webhook` subscriptions now DISPATCH.** The store was written and read by nothing:
  `subscribe` recorded a subscription and no runtime path ever delivered to it, so a subscription was a
  promise with no fulfillment. The model is now INBOUND (AD-042): the subscription claims a route under
  `/webhook/`, the external service posts to it, the agent handles the event, and the answer is delivered
  to the target the operator named (`--deliver platform:chat_id`) — because the inbound channel cannot
  carry a reply, so the answer used to be dropped on the floor. The route is the last path segment of the
  subscribed URL (`https://example.com/webhook/orders` claims `orders`), with `--route` to name one
  explicitly; a subscription that carries its own secret is verified against that secret. Two defects went
  with it: every webhook event was refused with "You are not authorized to use this bot." in any deployment
  that configures `admins` (the gate is a USER gate and a verified machine event has no user), and the
  agent's answer to an event was discarded by the inbound-only adapter.

- **iter-1131 — `operant gateway status` no longer claims the gateway is down while it is up.** The
  running check read one source, `~/.operant/gateway.pid`; a gateway run under systemd (the documented
  way to run it) has no such file, so the command printed "Runtime: not running" on a live box — and
  then deleted the pid file, leaving even less to go on. The check now consults the pid file AND the
  systemd unit, names the pid and its source, and only removes a stale pid file when neither source says
  the gateway is up.
- **iter-1130 — the doctor names WHICH `operant` copy is stale.** It used to report "2 copies of
  operant on PATH" and list both paths, leaving the operator to pick — and the wrong pick is a silent
  downgrade, because the shell resolves `~/.local/bin` first. Two copies of the same build were reported
  the same way although they are nothing to fix. The check now says which copy runs and which one is
  older, and treats identical copies as healthy.
- **iter-1129 — `operant doctor` stopped asking for a fix it cannot perform.** The skills-hub check
  warned "run: operant skills list" and listed "Initialize skills hub" among the issues to address — but
  that command does not create `~/.operant/skills/.hub`, and nothing else in the tree does either (the
  marketplace installs skills flat). On a machine with 103 working skills, the operator was told to fix
  something that could not be fixed. The check is now informational and never actionable.
- **iter-1128 — the second config surface is labelled, not silently live.** `operant_config::schema::Config`
  is never loaded in production (`Config::load_or_init` has no production caller), yet it holds 123 structs
  and 645 fields with no production reader outside its crate, so a setting added there looks live and is
  unreachable. The schema module now says so in its first lines, a static guard test fails if any
  non-test file starts loading the aggregate, and `scripts/audit-parallel-config.py` re-derives the counts.
  The retirement itself is planned, not executed: `docs/audit/2026-10-21-PARALLEL-CONFIG-SURFACE.md` and
  `docs/plans/plan-2026-10-21-config-schema-retirement.md`.
- **iter-1127 — `operant webhook test` really posts.** The command used to print "This would send a POST
  request with an HMAC-SHA256 signature to the webhook URL" and send nothing. The subscription store moved
  into core (`operant-core/src/gateway/webhook_subscriptions.rs`) with the signing scheme the inbound
  webhook adapter already accepts, and `operant webhook test` now posts the ping — signed with the
  subscription's secret when it has one — and reports the status and body. `operant webhook subscribe`
  now says plainly that the destination is recorded only: the runtime still does not dispatch events to a
  subscription, and the ledger row (RT-140) records the two candidate models and what each one still
  needs.
- **iter-1126 — the agent can read the operator's SOP playbooks.** `operant sop` read
  `~/.operant/sops/*.toml` with a loader private to the CLI; nothing else in the tree read that
  directory, so a playbook the harness could have followed sat unused. The store now lives in core
  (`operant-core/src/sops.rs`), the CLI shares that loader, and the agent got a `sop` tool (call it
  with no name to list the playbooks, with a name to read one) registered with the builtins.
- **iter-1125 — `operant hooks` is a real surface now.** The family read `~/.operant/data/hooks/*.hook`
  files that nothing wrote and nothing loaded, and the hook registry existed only when
  `[harness].enabled` was on — so in a normal install no hook could fire at all, while
  `operant hooks doctor` reported "All hooks are registered and ready". The store now lives in
  core (`operant-core/src/shell_hooks.rs`), the agent loads it when it starts, and the CLI can
  write it: `operant hooks add --event agent_end -- <command>`, `operant hooks test <name>` runs the
  command and reports its exit code, `operant hooks doctor` names the files it cannot use, and
  `operant hooks revoke <name>` removes one. Only the four events the agent actually emits are
  accepted (`agent_start`, `agent_end`, `turn_start`, `turn_end`) — a hook on an event that never
  fires is refused at write time and skipped at load time. The README's hook-parity line was
  corrected: there are no before/after-tool or memory-level hooks.
- **iter-1124 — `operant tools disable` now actually disables.** The CLI wrote `tool_state.json` next to
  the database and nothing read it: the runtime's disabled set came only from the TOML
  `[tools] disabled_tools`, so a tool the operator turned off kept reaching the agent while the
  command reported success. The state file now lives in core (`tools::tool_state`) and the registry
  build applies it, so the CLI and the agent read the same file. `operant tools list` also stopped
  hiding the disabled tools it was supposed to mark — it now prints every registered tool with a
  `[disabled]` marker and an "N registered, M enabled" count, and `operant tools enable` says when
  the TOML still disables the tool.
- **iter-1123 — accepted-but-unread config keys now say so, and the template stops advertising them.**
  `operant config show`, `operant.example.toml` and the loader all presented as live 19 settings nothing
  reads: the six `[tui]` presentation keys (`show_iterations`, `landing_title`, `prompt_placeholder`,
  `refresh_rate_ms`, `compact_width`, `medium_width`), `[skills] template_name`/`template_description`,
  the whole `[pk]` table (the live kernel surface is `[tools.kernel]`), `[bylaws] grant_ttl_days`
  (retired with the grant ledger — there is no `SeatApprover::mint` any more), the `[socialization]`
  table (phase 1 never landed), `[agent] max_consecutive_tool_only`, `[tools] event_channel_size` and
  `browser_binary_path`, `[memory] agentmemory_url`/`agentmemory_secret`, and
  `[tools.tool_search] search_default_limit`/`max_search_limit` (hardcoded 10/25 in the tool). The
  loader now prints one line per such key whose value differs from the default — a clean start stays
  silent — and the example template carries a comment in each place instead of a value. Nothing was
  deleted: `deny_unknown_fields` makes removal a breaking change for the operator's own config, which
  carries every one of these keys today. `scripts/audit-config-surface.py` re-derives the list instead
  of asking anyone to trust it.
- **iter-1122 — the configured speech-to-text model is the model that gets used.** `[tools.stt]
  groq_model` and `openai_model` were parsed, defaulted, printed by `operant config show`, and
  advertised in `operant.example.toml`, while `transcription_tool.rs` hardcoded
  `whisper-large-v3-turbo` and `whisper-1` at its two call sites — a provider retiring a model name
  was unfixable without a new build. Both sites now resolve per-call `model` → configured value →
  provider default, with the previous strings as the defaults. Found by a config-surface audit; the
  remaining 24 accepted-but-unread settings are triaged in the ledger (RT-136).
- **iter-1121 — `operant dashboard server --stop` and `--status` stop being stubs.** Both printed a
  sentence telling the operator to kill the process by hand. The server now records its pid and URL in
  `$OPERANT_HOME/dashboard.pid` before it starts serving, `--status` reports that server (and removes a
  stale file left by a crash), and `--stop` sends SIGTERM to the recorded pid and waits up to three
  seconds for it to exit so the port is free for the next start. A clean exit removes the file; a
  malformed file reads as "no server", never as a guessed pid, and pid 0 is refused outright —
  `kill(0, …)` addresses the caller's own process group, so a `0` in that file would have turned
  `--stop` into "SIGTERM my shell". The remaining accepted stub is `operant cron run <id>` (RT-135).

- **iter-1120 — the dashboard is one first-party page again, and it calls only the routes the server has.**
  The served frontend was a 1.9 MB build artefact from a larger harness: it called 79 `/api/...` paths
  against a four-route server and opened a `/api/ws` WebSocket that never existed, so every panel but
  four rendered blank (RT-44). `dashboard/index.html` is now a single self-contained file (inline CSS/JS,
  no fonts, no bundle step, no external requests) that renders `/api/status`, `/api/boards`, `/api/config`
  and refreshes every five seconds; the unused 2.2 MB of JS/CSS/fonts/images and the `/assets` route are
  gone. The contract is pinned by `the_page_calls_only_routes_the_server_serves`, which parses every
  `getJSON` path out of the page and requests it from the real server — a fetch without a route fails
  the test instead of blanking a panel. `/api/status` now nests its host metrics under `host` instead
  of `serde(flatten)`ing them into the top level: the flat shape existed only for the deleted bundle,
  and with it the new page read `host.cpu_count` from a payload that had no `host` object, so a live
  browser showed "n/a" for hostname, CPU, load, memory and disk while the API returned every value.
  The field test now encodes each value's ACCESS PATH (`host.cpu_count`, `s.version`), not just its
  name, and the browser check is how the mismatch was caught.

- **iter-1117 — a stale `OPENAI_BASE_URL` / `HERMES_MODEL` export can no longer win silently, and proxy environment variables no longer redirect provider traffic.** The loader records every env override, warns once per key, and `operant config show` marks the affected values; `HTTP(S)_PROXY` and `ALL_PROXY` are ignored by the provider client (a `[proxy]` table is the sanctioned path). A `~/.operant/config.toml` that the binary never reads, and a project-local `.env` that loses to `~/.operant/.env`, are now named in a one-time warning instead of being silently ignored. README documents the order: process env, then `.env` pins, then `operant.toml`.

- **iter-1108 — the agent's tools, the command-menu registration and the doctor's probe now use the CONFIGURED platform base, and the command-menu client uses the configured proxy.** A deployment behind a proxy could reach Telegram through the gateway and not through the agent's `send_message`, `react_to_message`, or the boot-time `setMyCommands` that registers the command menu. Each base also takes an environment override (`TELEGRAM_API_BASE`, `DISCORD_API_BASE`, `SLACK_API_BASE`, `WHATSAPP_API_BASE`) that wins over the config.

- **iter-1092 — the TUI's `/mcp` pane now paints a streamable-HTTP server.** It was in no transport group, so it was never shown — and it was counted as scrolled-off.

- **iter-1091 — the streaming send now records in the delivery ledger, so a streamed answer is not invisible in the delivery-health view.** `operant channel add sms` is no longer rejected, and `operant channel add webhook` no longer is either. The channel doctor now reports `"configured"` and `"health": "not probed"` instead of claiming a health it never checked, and `channel add` no longer tells the operator to verify connectivity with a command that does not reach the platform.

- **iter-1089 — `operant channel add` says when the channel it added is send-only.** "Added slack channel" led an operator to believe the agent would see Slack messages.

- **iter-1087 — `operant gateway status` now says which platforms can RECEIVE.** "Slack: enabled" led an operator to believe they could receive Slack messages; the status now says "send-only (no inbound)".

- **iter-1085 — `operant channel list` shows SMS.** The gateway's registry has seven adapters; the list showed six.

- **iter-1081 — the Slack streaming send chunks too.** `send_message_return_id` — the path the streaming path uses — still posted the whole body unchunked, so a long answer on Slack was rejected and lost while the plain send delivered it.

- **iter-1078 — the output fitter is sized against the current prompt, not the previous request's.** It could exceed the window by exactly the tool output accumulated since.

- **iter-1076 — the MCP SSE stream is no longer cut every 300 seconds.** The long-lived GET and the bounded POSTs now use separate clients, so the total timeout applies only to the calls.

- **iter-1075 — the WhatsApp and Twilio API bases are configurable.** They were hard-coded, so a deployment behind a proxy could reach Telegram, Discord and Slack but not those two.

- **iter-1072 — the Telegram voice send now uses the configured API base.** It hard-coded `api.telegram.org`, so a deployment pointing the API elsewhere had its voice messages go to the real Telegram API.

- **iter-1070 — an SMS over 1600 characters is now chunked instead of being lost.**

- **iter-1069 — a WhatsApp message over 4096 characters is now chunked instead of being lost.**

- **iter-1068 — a Slack message over 40,000 characters is now chunked instead of silently not delivered.** A debug print left in the send path is also removed.

- **iter-1067 — a Discord edit over the 2000-character limit no longer truncates the answer.** It now fails, so the full answer is sent as a fresh message instead of losing its tail silently.

- **iter-1066 — the gateway logs a delivery summary on stop.** The delivery ledger is in-memory in the daemon and nothing exposed it, so its counts — the whole point of the ledger — were invisible.

- **iter-1065 — the response-mirror feature is recorded as a gap.** It has a complete `hermes` parity implementation, no config key, and no production caller: it cannot be reached at all.

- **iter-1063 — Discord and Slack can now edit a sent message.** The streaming path's incremental updates were no-ops on both, so the user saw the first chunk of the answer and then the whole answer again as a second message.

- **iter-1062 — on Discord and Slack the agent's answer was truncated to its first chunk.** Only Telegram can edit a sent message, so the streaming path's edits were no-ops there, and the delivery marker suppressed the full send that would have delivered the rest.

- **iter-1061 — a Telegram permission prompt that failed to send is now a failure, not a silent stall.** It used to be reported as "no message id", which the caller reads as "the user never answered", so the turn waited forever for an approval that was never shown.

- **iter-1060 — the WhatsApp and SMS adapters name WHY a send failed, not just the status.** Both APIs put the reason in the response body, and both bodies were discarded.

- **iter-1059 — five proactive sends now log a delivery failure.** A permission prompt that could not be delivered left the turn blocked forever with no visible cause.

- **iter-1058 — the delivery ledger now records PROACTIVE sends, not just replies.** A scheduled message, a cron push or a permission notice was invisible in the operator's view of delivery health, and a failed one was invisible everywhere (most call sites discard the result).

- **iter-1056 — the Slack and Discord adapters now check the API's response.** They discarded it, so a rejected send (a revoked token, no channel access, a rate limit) was recorded as delivered. Slack needed more than a status check: it answers a rejected request with HTTP 200 and `ok: false`.

- **iter-1055 — the WhatsApp, Email and SMS adapters now say in their status report that they cannot receive.** Like Slack, they implement an inbound parser that nothing in the runtime invokes, so an operator saw "enabled" and believed messages would arrive.

- **iter-1053 — the test suite's environment locks are now crate-wide, async-aware and poison-tolerant.** Per-module locks did not exclude each other, so an interleaved `HOME`/`HERMES_HOME` or API-key mutation changed what another test saw — and one poisoned lock turned one failure into four.

- **iter-1053 — the profile tests no longer write into the operator's real `~/.operant/profiles/`.** They had been, since at least 2026-08-12: two profile directories with the test's exact content were found in the real home. The tests now assert the resolved root is their temp sandbox before writing.

- **iter-1053 — the test suite's environment lock is now crate-wide and poison-tolerant.** The per-module locks did not exclude each other, so an interleaved `HOME`/`HERMES_HOME` mutation changed the operant home mid-test; and one poisoned lock took four tests down with it.

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
