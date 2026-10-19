<!-- genome:zone records sha=997263e674f5 -->
**Decisions** (31)

| ID | Title | Status | Description |
|---|---|---|---|
| AD-001 | Body runs write no sessions row; attribution lives in the trajectory | Active | Body runs write no sessions row; attribution lives in the trajectory |
| AD-002 | The TUI org view reads the Holon read API in-process, not by shelling out | Active | The TUI org view reads the Holon read API in-process, not by shelling out |
| AD-003 | The TUI keeps the default operant home; HERMES_HOME belongs to the Holon driver | Active | The TUI keeps the default operant home; HERMES_HOME belongs to the Holon driver |
| AD-004 | Org mode validates /session employee ids against the Holon roster before binding | Active | Org mode validates /session employee ids against the Holon roster before binding |
| AD-005 | Operant plugins extend through one TOML manifest; permissions are enforced and tool and LLM hooks ship before a guest SDK | Proposed | Plugin ecosystem architecture: one TOML manifest, enforced permissions, tool-call and LLM-request hooks, config-owned enable state, and a git-first install path |
| AD-006 | Holon bodies are fenced from the supervisor's secrets and their slot | Active | A Holon body cannot read the supervisor's environment, cannot inherit chair authority, and dies with its slot |
| AD-007 | A Holon run without policy.json is refused, not run unfenced | Active | A Holon run whose policy.json is missing fails to start, instead of running with no seat authority |
| AD-008 | A chair notice is retried until delivered; driver tokens are plain | Active | The Holon kernel retries a failed chair notice on the next heartbeat, and refuses driver placeholders that are not plain tokens |
| AD-009 | A gateway budget is metered per seat turn, and a hard cap fails closed | Active | Spend is recorded per turn against the start-time seat and the window reads that ledger; a hard cap whose usage cannot be read refuses |
| AD-010 | A chat's conversation is kept on its row; /new and a seat change start a fresh one | Active | The conversation id persists on the session row, /new and a seat change start a fresh transcript, the continuation summary is keyed by it, and seat runs set their memory scope |
| AD-011 | A pending approval answers the key before any overlay | Active | An approval the agent is waiting on takes the key before any overlay, so a browser cannot swallow it |
| AD-012 | A rename is saved to the session, and a bare /rename waits for the list | Active | `/rename` persists the confirmed name, and a bare `/rename` opens the rename once the session list has loaded |
| AD-013 | A cleared delegation is abandoned, and the HUD follows the running one | Active | A delegation whose turn is cleared by /new or /rewind is marked abandoned, and the status bar shows a running delegation before a finished one |
| AD-014 | A shell-out restores the terminal before its result is reported, and releases mouse capture for the child | Active | A shell-out restores the terminal before it reports the child's exit, reports a failed or signalled exit as such, and frees the mouse for the child |
| AD-015 | The Ctrl+B fork tree is read off the event loop, and a database error is shown | Active | The branch browser's parent and fork rows are read on a blocking thread, and a failed read is reported in the transcript instead of opening an empty browser |
| AD-016 | A TTS engine start that panics is a tool error, and the release profile unwinds | Active | The Kokoro engine starts on its own task, so an ONNX Runtime panic becomes a tool error, and the release profile unwinds instead of aborting |
| AD-017 | ONNX Runtime is pinned to 1.24.4, and install refuses a library without API 24 | Active | The build and install scripts use ONNX Runtime 1.24.4, and install.sh refuses a runtime that does not serve API 24 |
| AD-018 | The doctor reports tool availability from configuration, flags duplicate binaries, and loads the config once | Active | operant doctor marks a tool available only through its configuration or a present env var, reports TTS as unverified, flags a second operant on PATH, and loads the config once |
| AD-019 | Gateway memory belongs to the named admins; other senders get their own bank | Active | A gateway turn is retained, and recalls the personal bank, only for a sender named in gateway.admins; any other unbound sender runs in a bank of its own |
| AD-020 | Knowledge writes need the user's approval; an unattended review cannot write memory | Active | memory_store, skill_manage and learning_manage prompt for approval like file writes, and the background review refuses memory writes and runs its skill guard |
| AD-021 | Stored knowledge is bounded, redacted, and written whole | Active | memory_store has size bounds, memory and skill text is redacted or refused for secrets, memory and skill files are replaced atomically, and skill paths refuse symlinks |
| AD-022 | Degraded memory is reported, and memory delete confirms like memory clear | Active | A memory store that fell back to memory is reported in the tool result and in memory stats, and memory delete asks for confirmation as memory clear already did |
| AD-023 | WASM plugins: enable state is config, plugins cannot replace built-ins, and HTTP stays off internal addresses | Active | The plugin enable state is a [plugins].disabled list in config, a plugin tool cannot take a built-in's name, a plugin's HTTP request cannot reach an internal address, and the memory plugin's tool sche |
| AD-024 | A [keybindings] table loads and is warned about; dispatch does not read it yet | Active | A [keybindings] table in operant.toml is accepted and produces one warning, instead of failing the whole config load |
| AD-025 | The settings.json hooks menu is removed; /hooks points to operant hooks | Active | The read-only settings.json hooks menu is removed because no runtime path reads or fires those hooks; /hooks names the command that manages shell hooks |
| AD-026 | The git diff loads on a worker thread, and the viewer shows a reading state | Active | The git diff for the diff viewer is read on a worker thread and taken on the event-loop tick, so opening /diff does not stall the loop |
| AD-027 | The autonomous loop commits only the paths its tick changed | Active | The autonomous publish stages and commits only the paths this tick made dirty, so a file the owner left uncommitted is not published |
| AD-028 | A new org-mode chat starts unbound; no default employee is set | Active | New org-mode chats start unbound, with no default employee, which is the current behaviour and the recommended option for the seat-default question |
| AD-029 | Context management stays on the synchronous summarize rung; agent-driven reduction is not built | Active | The magic-context review's agent-driven reduction and background historian are not built now; the synchronous summarize rung stays |
| AD-030 | The Telegram token filter is covered for URL-embedded tokens; the historic log is not rewritten | Active | Telegram bot tokens in bot URLs are redacted by a tested rule; the old gateway.log lines are not rewritten, and token rotation stays the owner's call |
| AD-031 | Dead TUI code is removed where it is ours; verbatim ports, the keybinding registry and vendored style modules stay allowlisted | Active | The clippy allowlist's dead-code entries drop from 52 to 47: unused TUI items with no caller are removed, and the verbatim jcode ports, the keybinding registry and the vendored style modules keep thei |
<!-- genome:end records -->

<!-- genome:zone idmap sha=a7a72b4c9dd0 -->
(no retired records)
<!-- genome:end idmap -->
