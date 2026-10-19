<!-- genome:zone records sha=eb488a22381d -->
**Decisions** (22)

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
<!-- genome:end records -->

<!-- genome:zone idmap sha=a7a72b4c9dd0 -->
(no retired records)
<!-- genome:end idmap -->
